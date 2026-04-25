//! Interactive cyberpunk chat TUI — `phantom chat`.
//!
//! Uses the same `phantomchat_core::SessionStore` + `phantomchat_relays`
//! plumbing as the headless `send`/`listen` commands; only the surface is
//! different. A single relay subscribe-task feeds incoming envelopes into
//! an mpsc channel; the render loop drains it, runs each through the local
//! ratchet, and appends decrypts to the unified message stream. Outbound
//! messages take the same `SessionStore::send` → `relay.publish` path,
//! then echo into the stream for instant visual feedback.

use std::{
    fs,
    io::{self, Stdout},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::STANDARD as B64};

use anyhow::{anyhow, Context};
use chrono::Local;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, KeyEvent,
        KeyEventKind, KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt;
use phantomchat_core::{
    address::PhantomAddress,
    keys::{PhantomSigningKey, SpendKey, ViewKey},
    privacy::{PrivacyConfig, PrivacyMode},
    session::SessionStore,
};
use phantomchat_relays::{make_relay, BridgeProvider};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame, Terminal,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, Mutex};
use x25519_dalek::{PublicKey, StaticSecret};

// ── Cyberpunk palette (matches the CLI banner / Flutter app) ─────────────────

const NEON_GREEN: Color = Color::Rgb(0, 255, 159);
const NEON_MAGENTA: Color = Color::Rgb(255, 0, 255);
const CYBER_CYAN: Color = Color::Rgb(0, 255, 255);
const DIM_GREEN: Color = Color::Rgb(0, 130, 80);
const SOFT_GREY: Color = Color::Rgb(150, 150, 160);

// ── On-disk contacts file ────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Contact {
    pub label: String,
    pub address: String,
    /// Hex-encoded Ed25519 public key used by sealed-sender attribution.
    /// `None` until the user binds it via the `b` keybinding (or sets it
    /// out-of-band). Optional + skip-if-none to keep on-disk format
    /// backwards-compatible with pre-attribution contact files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signing_pub: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ContactBook {
    #[serde(default)]
    pub contacts: Vec<Contact>,
}

impl ContactBook {
    fn load(path: &Path) -> Self {
        fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn save(&self, path: &Path) -> io::Result<()> {
        fs::write(path, serde_json::to_vec_pretty(self).expect("serialize contactbook"))
    }
}

// ── Message-stream entries ───────────────────────────────────────────────────

#[derive(Clone, Debug)]
enum MsgKind {
    Incoming, // from the relay (any peer; sender unknown without sealed-sender)
    Outgoing, // we sent it to a specific contact
    System,   // info / status / errors
}

#[derive(Clone, Debug)]
struct MsgLine {
    ts: String,
    kind: MsgKind,
    label: String, // contact label, "INBOX", "you", or "·"
    body: String,
}

impl MsgLine {
    fn now(kind: MsgKind, label: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            ts: Local::now().format("%H:%M:%S").to_string(),
            kind,
            label: label.into(),
            body: body.into(),
        }
    }
}

// ── UI events from the background tasks ──────────────────────────────────────

enum UiEvent {
    /// Decrypted message with optional sealed-sender attribution.
    /// `sender_pub` = raw Ed25519 verifying-key bytes (from
    /// `SealedSender::sender_pub`); `sig_ok` is the signature-verification
    /// result. `None` means the envelope had no sealed-sender attached.
    Decrypted {
        plaintext: Vec<u8>,
        sender_pub: Option<[u8; 32]>,
        sig_ok: bool,
    },
    NotForUs,
    Sent { label: String, body: String },
    /// Reserved for surfacing background-task status messages in the
    /// chat stream (e.g. relay reconnects). Not yet wired to a producer.
    #[allow(dead_code)]
    Status(String),
    Error(String),
}

// ── Focus / modal state ──────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Focus {
    Contacts,
    Input,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Modal {
    None,
    AddContact { label: String, address: String, field: AddField },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AddField {
    Label,
    Address,
}

// ── App state ────────────────────────────────────────────────────────────────

struct App {
    // Retained on App for symmetry with the headless commands and future
    // "rotate identity" / "re-scan with this view key" features. Not read
    // by the current render loop — backing tasks were spawned with copies.
    #[allow(dead_code)]
    keyfile: PathBuf,
    contacts_path: PathBuf,
    sessions_path: PathBuf,
    relay_url: String,
    mode_label: String,

    #[allow(dead_code)]
    view_key: ViewKey,
    #[allow(dead_code)]
    spend_key: SpendKey,
    signing_key: PhantomSigningKey,

    contacts: ContactBook,
    contacts_state: ListState,

    /// Most-recently seen sealed-sender pubkey that did NOT match any
    /// contact's `signing_pub`. Pressing `b` while a contact is selected
    /// binds this pubkey to that contact.
    last_unbound_sender: Option<[u8; 32]>,

    messages: Vec<MsgLine>,
    msg_scroll: u16,

    input: String,
    focus: Focus,
    modal: Modal,
    should_quit: bool,

    // shared with background tasks
    store: Arc<Mutex<SessionStore>>,
    relay: Arc<dyn BridgeProvider>,
    out_tx: mpsc::UnboundedSender<UiEvent>,

    // total envelopes seen (for "scanned N · decrypted M" footer)
    seen: u64,
    decrypted: u64,
}

impl App {
    fn active_contact(&self) -> Option<&Contact> {
        self.contacts_state
            .selected()
            .and_then(|i| self.contacts.contacts.get(i))
    }

    fn push_system(&mut self, body: impl Into<String>) {
        self.messages.push(MsgLine::now(MsgKind::System, "·", body));
    }
}

// ── Public entrypoint ────────────────────────────────────────────────────────

pub async fn run_chat(
    keyfile: PathBuf,
    relay_url: String,
    cfg: &PrivacyConfig,
) -> anyhow::Result<()> {
    // ── Load identity ────────────────────────────────────────────────────────
    let (view_key, spend_key, signing_key, signing_was_upgraded) =
        load_identity(&keyfile)?;

    let sessions_path = sessions_path_for(&keyfile);
    let contacts_path = contacts_path_for(&keyfile);

    let store = SessionStore::load(&sessions_path)
        .with_context(|| format!("loading sessions from {}", sessions_path.display()))?;
    let store = Arc::new(Mutex::new(store));

    let contacts = ContactBook::load(&contacts_path);
    let mut contacts_state = ListState::default();
    if !contacts.contacts.is_empty() {
        contacts_state.select(Some(0));
    }

    // ── Relay handle ────────────────────────────────────────────────────────
    let stealth = cfg.mode == PrivacyMode::MaximumStealth;
    let proxy = cfg.proxy_addr();
    let relay: Arc<dyn BridgeProvider> = Arc::from(make_relay(&relay_url, stealth, proxy));

    let mode_label = match cfg.mode {
        PrivacyMode::DailyUse => "DAILY USE".to_string(),
        PrivacyMode::MaximumStealth => "MAXIMUM STEALTH".to_string(),
    };

    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<UiEvent>();

    // Spawn the relay subscriber. It dumps every envelope into the local
    // SessionStore::receive pipeline; results (plaintext / not-for-us /
    // error) are forwarded to the UI as UiEvents.
    {
        let store = Arc::clone(&store);
        let view_key = view_key.clone();
        let spend_key = spend_key.clone();
        let save_path = sessions_path.clone();
        let out_tx_handler = out_tx.clone();
        let out_tx_init = out_tx.clone();
        let relay = Arc::clone(&relay);

        let handler: phantomchat_relays::EnvelopeHandler = Box::new(move |env| {
            let store = Arc::clone(&store);
            let view_key = view_key.clone();
            let spend_key = spend_key.clone();
            let save_path = save_path.clone();
            let out_tx = out_tx_handler.clone();
            tokio::spawn(async move {
                let mut guard = store.lock().await;
                match guard.receive_full(&env, &view_key, &spend_key, None) {
                    Ok(Some(msg)) => {
                        let _ = guard.save(&save_path);
                        let (sender_pub, sig_ok) = match msg.sender {
                            Some((attr, ok)) => (Some(attr.sender_pub), ok),
                            None => (None, false),
                        };
                        let _ = out_tx.send(UiEvent::Decrypted {
                            plaintext: msg.plaintext,
                            sender_pub,
                            sig_ok,
                        });
                    }
                    Ok(None) => {
                        let _ = out_tx.send(UiEvent::NotForUs);
                    }
                    Err(e) => {
                        let _ = out_tx.send(UiEvent::Error(format!("decrypt: {}", e)));
                    }
                }
            });
        });

        tokio::spawn(async move {
            if let Err(e) = relay.subscribe(handler).await {
                let _ = out_tx_init.send(UiEvent::Error(format!("subscribe: {}", e)));
            }
        });
    }

    // ── App state ───────────────────────────────────────────────────────────
    let mut app = App {
        keyfile,
        contacts_path,
        sessions_path,
        relay_url: relay_url.clone(),
        mode_label,
        view_key,
        spend_key,
        signing_key,
        contacts,
        contacts_state,
        last_unbound_sender: None,
        messages: Vec::new(),
        msg_scroll: 0,
        input: String::new(),
        focus: Focus::Input,
        modal: Modal::None,
        should_quit: false,
        store,
        relay,
        out_tx,
        seen: 0,
        decrypted: 0,
    };

    app.push_system(format!(
        "Connected · relay {} · mode {} · {} contact(s) loaded",
        relay_url,
        app.mode_label,
        app.contacts.contacts.len()
    ));
    if signing_was_upgraded {
        app.push_system(
            "Upgraded keyfile: generated fresh Ed25519 signing key for sealed-sender attribution.",
        );
    }
    if app.contacts.contacts.is_empty() {
        app.push_system("Press Ctrl+N to add a contact (label + phantom: address).");
    }

    // ── Terminal setup ──────────────────────────────────────────────────────
    let mut terminal = setup_terminal()?;
    let mut events = EventStream::new();

    let result = event_loop(&mut app, &mut terminal, &mut events, &mut out_rx).await;

    restore_terminal(&mut terminal)?;
    result
}

// ── Event loop ───────────────────────────────────────────────────────────────

async fn event_loop(
    app: &mut App,
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    events: &mut EventStream,
    out_rx: &mut mpsc::UnboundedReceiver<UiEvent>,
) -> anyhow::Result<()> {
    // First draw before any events arrive.
    terminal.draw(|f| draw(f, app))?;

    let mut tick = tokio::time::interval(Duration::from_millis(250));

    loop {
        tokio::select! {
            biased;

            maybe_key = events.next() => {
                match maybe_key {
                    Some(Ok(Event::Key(k))) if k.kind == KeyEventKind::Press => {
                        handle_key(app, k).await;
                    }
                    Some(Ok(Event::Resize(_, _))) => {}
                    Some(Err(e)) => {
                        app.push_system(format!("input error: {}", e));
                    }
                    Some(_) => {}
                    None => {
                        // event stream ended — terminal closed
                        break;
                    }
                }
            }

            Some(ev) = out_rx.recv() => {
                handle_ui_event(app, ev);
            }

            _ = tick.tick() => {
                // periodic redraw — keeps the "scanned N" counter live even
                // if no key has been pressed.
            }
        }

        if app.should_quit {
            break;
        }
        terminal.draw(|f| draw(f, app))?;
    }

    Ok(())
}

fn handle_ui_event(app: &mut App, ev: UiEvent) {
    match ev {
        UiEvent::Decrypted { plaintext, sender_pub, sig_ok } => {
            app.seen = app.seen.saturating_add(1);
            app.decrypted = app.decrypted.saturating_add(1);
            let body = String::from_utf8_lossy(&plaintext).to_string();

            // Resolve label from sealed-sender attribution.
            //   - None              → no sealed-sender on the wire → "INBOX"
            //   - Some, !sig_ok     → tampered → "INBOX!" (don't trust pub)
            //   - Some + matched    → contact label
            //   - Some + unmatched  → "?<8-hex-prefix>" + remember as
            //     last_unbound_sender so user can press `b` to bind it.
            let label = match sender_pub {
                None => "INBOX".to_string(),
                Some(_) if !sig_ok => "INBOX!".to_string(),
                Some(pub_bytes) => {
                    let pub_hex = hex::encode(pub_bytes);
                    let matched = app
                        .contacts
                        .contacts
                        .iter()
                        .find(|c| {
                            c.signing_pub
                                .as_deref()
                                .map(|h| h.eq_ignore_ascii_case(&pub_hex))
                                .unwrap_or(false)
                        })
                        .map(|c| c.label.clone());
                    match matched {
                        Some(lbl) => lbl,
                        None => {
                            app.last_unbound_sender = Some(pub_bytes);
                            format!("?{}", &pub_hex[..8])
                        }
                    }
                }
            };
            app.messages
                .push(MsgLine::now(MsgKind::Incoming, label, body));
        }
        UiEvent::NotForUs => {
            app.seen = app.seen.saturating_add(1);
        }
        UiEvent::Sent { label, body } => {
            app.messages
                .push(MsgLine::now(MsgKind::Outgoing, label, body));
        }
        UiEvent::Status(s) => {
            app.push_system(s);
        }
        UiEvent::Error(s) => {
            app.messages
                .push(MsgLine::now(MsgKind::System, "ERR", s));
        }
    }
}

// ── Key handling ─────────────────────────────────────────────────────────────

async fn handle_key(app: &mut App, k: KeyEvent) {
    // Modal first — eats all input until dismissed.
    if !matches!(app.modal, Modal::None) {
        handle_modal_key(app, k);
        return;
    }

    // Global keys
    if k.modifiers.contains(KeyModifiers::CONTROL) {
        match k.code {
            KeyCode::Char('c') | KeyCode::Char('q') => {
                app.should_quit = true;
                return;
            }
            KeyCode::Char('n') => {
                app.modal = Modal::AddContact {
                    label: String::new(),
                    address: String::new(),
                    field: AddField::Label,
                };
                return;
            }
            _ => {}
        }
    }

    if matches!(k.code, KeyCode::Esc) {
        app.should_quit = true;
        return;
    }

    if matches!(k.code, KeyCode::Tab) {
        app.focus = match app.focus {
            Focus::Input => Focus::Contacts,
            Focus::Contacts => Focus::Input,
        };
        return;
    }

    match app.focus {
        Focus::Contacts => handle_contacts_key(app, k),
        Focus::Input => handle_input_key(app, k).await,
    }
}

fn handle_contacts_key(app: &mut App, k: KeyEvent) {
    let n = app.contacts.contacts.len();
    if n == 0 {
        return;
    }
    match k.code {
        KeyCode::Up => {
            let i = app.contacts_state.selected().unwrap_or(0);
            app.contacts_state
                .select(Some(if i == 0 { n - 1 } else { i - 1 }));
        }
        KeyCode::Down => {
            let i = app.contacts_state.selected().unwrap_or(0);
            app.contacts_state.select(Some((i + 1) % n));
        }
        KeyCode::Enter => {
            // Select + jump to input
            app.focus = Focus::Input;
        }
        KeyCode::Char('b') => {
            // Bind the most-recent unbound sender_pub to the selected contact.
            let Some(idx) = app.contacts_state.selected() else { return; };
            let Some(pub_bytes) = app.last_unbound_sender else {
                app.push_system(
                    "no unbound sender yet — wait for an incoming sealed message tagged ?<hex>",
                );
                return;
            };
            let pub_hex = hex::encode(pub_bytes);
            let label = {
                let c = &mut app.contacts.contacts[idx];
                c.signing_pub = Some(pub_hex.clone());
                c.label.clone()
            };
            if let Err(e) = app.contacts.save(&app.contacts_path) {
                app.push_system(format!("failed to persist contacts: {}", e));
                return;
            }
            app.last_unbound_sender = None;
            app.push_system(format!(
                "bound '{}' ↔ signing_pub {}…",
                label,
                &pub_hex[..8]
            ));
        }
        KeyCode::Delete => {
            if let Some(i) = app.contacts_state.selected() {
                let removed = app.contacts.contacts.remove(i);
                let _ = app.contacts.save(&app.contacts_path);
                app.push_system(format!("removed contact '{}'", removed.label));
                let new_n = app.contacts.contacts.len();
                if new_n == 0 {
                    app.contacts_state.select(None);
                } else {
                    app.contacts_state.select(Some(i.min(new_n - 1)));
                }
            }
        }
        _ => {}
    }
}

async fn handle_input_key(app: &mut App, k: KeyEvent) {
    match k.code {
        KeyCode::Enter => {
            if app.input.trim().is_empty() {
                return;
            }
            if let Some(contact) = app.active_contact().cloned() {
                let body = std::mem::take(&mut app.input);
                spawn_send(app, contact, body);
            } else {
                app.push_system("no active contact — Ctrl+N to add one");
                app.focus = Focus::Contacts;
            }
        }
        KeyCode::Backspace => {
            app.input.pop();
        }
        KeyCode::Char(c) => {
            app.input.push(c);
        }
        KeyCode::PageUp => {
            app.msg_scroll = app.msg_scroll.saturating_add(5);
        }
        KeyCode::PageDown => {
            app.msg_scroll = app.msg_scroll.saturating_sub(5);
        }
        _ => {}
    }
}

fn handle_modal_key(app: &mut App, k: KeyEvent) {
    let Modal::AddContact { label, address, field } = &mut app.modal else {
        return;
    };
    match k.code {
        KeyCode::Esc => {
            app.modal = Modal::None;
        }
        KeyCode::Tab => {
            *field = match field {
                AddField::Label => AddField::Address,
                AddField::Address => AddField::Label,
            };
        }
        KeyCode::Enter => {
            // Commit if both fields look reasonable.
            let label_v = label.trim().to_string();
            let address_v = address.trim().to_string();
            if label_v.is_empty() || address_v.is_empty() {
                return;
            }
            match PhantomAddress::parse(&address_v) {
                Some(_) => {
                    app.contacts.contacts.push(Contact {
                        label: label_v.clone(),
                        address: address_v,
                        signing_pub: None,
                    });
                    let _ = app.contacts.save(&app.contacts_path);
                    if app.contacts_state.selected().is_none() {
                        app.contacts_state.select(Some(0));
                    }
                    app.modal = Modal::None;
                    app.push_system(format!("added contact '{}'", label_v));
                }
                None => {
                    *address = "<bad address — expected phantom:view:spend>".to_string();
                }
            }
        }
        KeyCode::Backspace => match field {
            AddField::Label => {
                label.pop();
            }
            AddField::Address => {
                address.pop();
            }
        },
        KeyCode::Char(c) => match field {
            AddField::Label => label.push(c),
            AddField::Address => address.push(c),
        },
        _ => {}
    }
}

// ── Send path ────────────────────────────────────────────────────────────────

fn spawn_send(app: &App, contact: Contact, body: String) {
    let store = Arc::clone(&app.store);
    let relay = Arc::clone(&app.relay);
    let sessions_path = app.sessions_path.clone();
    let out_tx = app.out_tx.clone();
    let signing_key = app.signing_key.clone();

    tokio::spawn(async move {
        let recipient = match PhantomAddress::parse(&contact.address) {
            Some(r) => r,
            None => {
                let _ = out_tx.send(UiEvent::Error(format!(
                    "bad address for {}",
                    contact.label
                )));
                return;
            }
        };

        let envelope = {
            let mut guard = store.lock().await;
            // PoW difficulty 8 — instant on a desktop, still > zero on the
            // wire so the relay sees a non-trivial Hashcash. The headless
            // CLI uses 16; keep this light to avoid the chat feeling laggy.
            // `send_sealed` stamps the envelope with our Ed25519 signature
            // so peers can attribute (and the receiver can match against
            // their contact list).
            let env = guard.send_sealed(&recipient, body.as_bytes(), &signing_key, 8);
            let _ = guard.save(&sessions_path);
            env
        };

        match relay.publish(envelope).await {
            Ok(()) => {
                let _ = out_tx.send(UiEvent::Sent {
                    label: contact.label,
                    body,
                });
            }
            Err(e) => {
                let _ = out_tx.send(UiEvent::Error(format!("publish: {}", e)));
            }
        }
    });
}

// ── Render ───────────────────────────────────────────────────────────────────

fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();

    // Outer chrome ────────────────────────────────────────────────────────────
    let outer = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(DIM_GREEN))
        .title(Line::from(vec![
            Span::styled(" PHANTOMCHAT ", Style::default().fg(NEON_GREEN).add_modifier(Modifier::BOLD)),
            Span::styled("│ ", Style::default().fg(DIM_GREEN)),
            Span::styled(format!("[ {} ]", app.mode_label), Style::default().fg(NEON_MAGENTA)),
        ]));
    let inner = outer.inner(area);
    f.render_widget(outer, area);

    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),     // main row
            Constraint::Length(3),  // input
            Constraint::Length(1),  // footer
        ])
        .split(inner);

    let h = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(28), // contacts
            Constraint::Min(40),    // messages
        ])
        .split(v[0]);

    draw_contacts(f, app, h[0]);
    draw_messages(f, app, h[1]);
    draw_input(f, app, v[1]);
    draw_footer(f, app, v[2]);

    if let Modal::AddContact { label, address, field } = &app.modal {
        draw_add_contact_modal(f, area, label, address, *field);
    }
}

fn draw_contacts(f: &mut Frame, app: &mut App, area: Rect) {
    let title = format!(
        " CONTACTS ({}/{}) ",
        app.contacts_state.selected().map(|i| i + 1).unwrap_or(0),
        app.contacts.contacts.len()
    );
    let border_color = if app.focus == Focus::Contacts { NEON_GREEN } else { DIM_GREEN };

    let items: Vec<ListItem> = if app.contacts.contacts.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "  (no contacts — Ctrl+N to add)",
            Style::default().fg(SOFT_GREY),
        )))]
    } else {
        app.contacts
            .contacts
            .iter()
            .map(|c| {
                let short = short_address(&c.address);
                ListItem::new(vec![
                    Line::from(vec![
                        Span::styled("  ", Style::default()),
                        Span::styled(c.label.clone(), Style::default().fg(NEON_GREEN).add_modifier(Modifier::BOLD)),
                    ]),
                    Line::from(vec![
                        Span::styled("  ", Style::default()),
                        Span::styled(short, Style::default().fg(SOFT_GREY)),
                    ]),
                ])
            })
            .collect()
    };

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .title(Span::styled(title, Style::default().fg(NEON_GREEN).add_modifier(Modifier::BOLD))),
        )
        .highlight_style(
            Style::default()
                .bg(Color::Rgb(20, 40, 30))
                .fg(NEON_GREEN)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    f.render_stateful_widget(list, area, &mut app.contacts_state);
}

fn draw_messages(f: &mut Frame, app: &App, area: Rect) {
    let active = app
        .active_contact()
        .map(|c| format!(" #{} ", c.label))
        .unwrap_or_else(|| " #stream ".to_string());

    let border_color = if app.focus == Focus::Input || app.focus == Focus::Contacts {
        DIM_GREEN
    } else {
        NEON_GREEN
    };

    let mut lines: Vec<Line> = Vec::with_capacity(app.messages.len());
    for m in &app.messages {
        let (arrow, color) = match m.kind {
            MsgKind::Incoming => ("◀", CYBER_CYAN),
            MsgKind::Outgoing => ("▶", NEON_GREEN),
            MsgKind::System => ("·", SOFT_GREY),
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{}  ", m.ts), Style::default().fg(SOFT_GREY)),
            Span::styled(format!("{}  ", arrow), Style::default().fg(color).add_modifier(Modifier::BOLD)),
            Span::styled(
                format!("{:<10}", truncate(&m.label, 10)),
                Style::default().fg(NEON_MAGENTA),
            ),
            Span::raw("  "),
            Span::styled(m.body.clone(), Style::default().fg(Color::Rgb(220, 240, 230))),
        ]));
    }

    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (waiting for first message — type below or wait for incoming)",
            Style::default().fg(SOFT_GREY),
        )));
    }

    // Scroll: msg_scroll counts lines from the bottom.
    let total = lines.len() as u16;
    let visible = area.height.saturating_sub(2);
    let max_offset = total.saturating_sub(visible);
    let from_top = max_offset.saturating_sub(app.msg_scroll.min(max_offset));

    let para = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .title(Span::styled(active, Style::default().fg(CYBER_CYAN).add_modifier(Modifier::BOLD))),
        )
        .wrap(Wrap { trim: false })
        .scroll((from_top, 0));

    f.render_widget(para, area);
}

fn draw_input(f: &mut Frame, app: &App, area: Rect) {
    let title = match app.active_contact() {
        Some(c) => format!(" → {} ", c.label),
        None => " → (no contact selected) ".to_string(),
    };
    let border_color = if app.focus == Focus::Input { NEON_GREEN } else { DIM_GREEN };

    let para = Paragraph::new(Line::from(vec![
        Span::styled("» ", Style::default().fg(NEON_MAGENTA).add_modifier(Modifier::BOLD)),
        Span::styled(app.input.clone(), Style::default().fg(NEON_GREEN)),
        Span::styled("█", Style::default().fg(NEON_GREEN).add_modifier(Modifier::SLOW_BLINK)),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .title(Span::styled(title, Style::default().fg(NEON_GREEN))),
    );

    f.render_widget(para, area);
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let stream = format!(
        "scanned {} · decrypted {}",
        app.seen, app.decrypted
    );
    let footer = Line::from(vec![
        Span::styled(" Tab", Style::default().fg(NEON_GREEN).add_modifier(Modifier::BOLD)),
        Span::styled(" focus  │ ", Style::default().fg(SOFT_GREY)),
        Span::styled("Ctrl+N", Style::default().fg(NEON_GREEN).add_modifier(Modifier::BOLD)),
        Span::styled(" add  │ ", Style::default().fg(SOFT_GREY)),
        Span::styled("Del", Style::default().fg(NEON_GREEN).add_modifier(Modifier::BOLD)),
        Span::styled(" remove  │ ", Style::default().fg(SOFT_GREY)),
        Span::styled("b", Style::default().fg(NEON_GREEN).add_modifier(Modifier::BOLD)),
        Span::styled(" bind  │ ", Style::default().fg(SOFT_GREY)),
        Span::styled("Esc", Style::default().fg(NEON_GREEN).add_modifier(Modifier::BOLD)),
        Span::styled(" quit  │ ", Style::default().fg(SOFT_GREY)),
        Span::styled(stream, Style::default().fg(CYBER_CYAN)),
        Span::styled("  │ ", Style::default().fg(SOFT_GREY)),
        Span::styled(short_relay(&app.relay_url), Style::default().fg(NEON_MAGENTA)),
        Span::raw(" "),
    ]);
    f.render_widget(Paragraph::new(footer), area);
}

fn draw_add_contact_modal(
    f: &mut Frame,
    area: Rect,
    label: &str,
    address: &str,
    field: AddField,
) {
    let modal_area = centered_rect(70, 30, area);
    f.render_widget(Clear, modal_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(NEON_MAGENTA))
        .title(Span::styled(
            " ADD CONTACT ",
            Style::default().fg(NEON_MAGENTA).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(modal_area);
    f.render_widget(block, modal_area);

    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .margin(1)
        .split(inner);

    f.render_widget(
        Paragraph::new("Tab: switch field    Enter: save    Esc: cancel")
            .style(Style::default().fg(SOFT_GREY)),
        v[0],
    );

    let label_border = if field == AddField::Label { NEON_GREEN } else { DIM_GREEN };
    f.render_widget(
        Paragraph::new(label.to_string())
            .style(Style::default().fg(NEON_GREEN))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(label_border))
                    .title(" Label "),
            ),
        v[1],
    );

    let addr_border = if field == AddField::Address { NEON_GREEN } else { DIM_GREEN };
    f.render_widget(
        Paragraph::new(address.to_string())
            .style(Style::default().fg(NEON_GREEN))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(addr_border))
                    .title(" Address (phantom:view:spend) "),
            ),
        v[3],
    );
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(v[1])[1]
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn short_address(s: &str) -> String {
    // phantom:<view>:<spend>  → phantom:abcd…wxyz
    let stripped = s.strip_prefix("phantom:").or_else(|| s.strip_prefix("phantomx:")).unwrap_or(s);
    let prefix = if s.starts_with("phantomx:") { "phantomx:" } else { "phantom:" };
    let head: String = stripped.chars().take(6).collect();
    let tail: String = stripped.chars().rev().take(4).collect::<String>().chars().rev().collect();
    format!("{}{}…{}", prefix, head, tail)
}

fn short_relay(url: &str) -> String {
    url.replace("wss://", "").replace("ws://", "")
}

pub fn sessions_path_for(keyfile: &Path) -> PathBuf {
    let parent = keyfile.parent().unwrap_or_else(|| Path::new("."));
    let stem = keyfile.file_stem().and_then(|s| s.to_str()).unwrap_or("keys");
    parent.join(format!("{}.sessions.json", stem))
}

fn contacts_path_for(keyfile: &Path) -> PathBuf {
    let parent = keyfile.parent().unwrap_or_else(|| Path::new("."));
    let stem = keyfile.file_stem().and_then(|s| s.to_str()).unwrap_or("keys");
    parent.join(format!("{}.contacts.json", stem))
}

/// Load view + spend + signing keys from the keyfile.
///
/// Backwards-compat: if `signing_private` is absent (pre-attribution
/// keyfiles generated before this feature shipped), a fresh
/// [`PhantomSigningKey`] is generated and the keyfile is rewritten with
/// the new fields. The fourth tuple element signals whether such an
/// upgrade happened so the UI can surface a system message.
fn load_identity(
    file: &Path,
) -> anyhow::Result<(ViewKey, SpendKey, PhantomSigningKey, bool)> {
    let raw = fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    let mut json: serde_json::Value = serde_json::from_slice(&raw)?;

    let view_bytes = B64.decode(
        json["view_private"].as_str().context("missing view_private")?,
    )?;
    let view_secret = StaticSecret::from(
        <[u8; 32]>::try_from(view_bytes.as_slice())
            .map_err(|_| anyhow!("bad view key"))?,
    );
    let view_key = ViewKey {
        public: PublicKey::from(&view_secret),
        secret: view_secret,
    };

    let spend_bytes = B64.decode(
        json["spend_private"].as_str().context("missing spend_private")?,
    )?;
    let spend_secret = StaticSecret::from(
        <[u8; 32]>::try_from(spend_bytes.as_slice())
            .map_err(|_| anyhow!("bad spend key"))?,
    );
    let spend_key = SpendKey {
        public: PublicKey::from(&spend_secret),
        secret: spend_secret,
    };

    let (signing_key, upgraded) = match json["signing_private"].as_str() {
        Some(s) => {
            let bytes = B64.decode(s)?;
            let arr: [u8; 32] = <[u8; 32]>::try_from(bytes.as_slice())
                .map_err(|_| anyhow!("bad signing key"))?;
            (PhantomSigningKey::from_bytes(arr), false)
        }
        None => {
            let sk = PhantomSigningKey::generate();
            // Persist so subsequent runs use the same identity for
            // attribution. Best-effort: a write failure is non-fatal —
            // the in-memory key still works for this session.
            if let Some(obj) = json.as_object_mut() {
                obj.insert(
                    "signing_private".to_string(),
                    serde_json::Value::String(B64.encode(sk.to_bytes())),
                );
                obj.insert(
                    "signing_public".to_string(),
                    serde_json::Value::String(hex::encode(sk.public_bytes())),
                );
                let _ = serde_json::to_vec_pretty(&json)
                    .ok()
                    .and_then(|b| fs::write(file, b).ok());
            }
            (sk, true)
        }
    };

    Ok((view_key, spend_key, signing_key, upgraded))
}

// ── Terminal lifecycle ───────────────────────────────────────────────────────

fn setup_terminal() -> anyhow::Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    Ok(Terminal::new(backend)?)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> anyhow::Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

// Suppress unused-import warnings if downstream modules trim things.
#[allow(dead_code)]
fn _imports_used() {
    let _ = event::poll;
}
