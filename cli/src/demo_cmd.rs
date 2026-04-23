//! `phantom demo` — run a quick crypto showcase in-process.
//!
//! Spins up 3 ephemeral identities, creates a Sender-Keys group, exchanges
//! sender keys, and round-trips a few messages. Verifies the crypto stack
//! end-to-end with zero setup (no network, no disk files, no keygen).

use anyhow::{anyhow, Result};
use colored::Colorize;
use phantomchat_core::{
    address::PhantomAddress,
    group::PhantomGroup,
    keys::{IdentityKey, PhantomSigningKey, SpendKey, ViewKey},
};

fn h(s: &str) {
    println!("\n{}", format!("▸ {}", s).cyan().bold());
}
fn ok(s: &str) {
    println!("  {} {}", "✓".green().bold(), s);
}
fn detail(label: &str, value: &str) {
    println!("    {} {}", format!("{:12}", label).dimmed(), value);
}

struct Persona {
    name: &'static str,
    identity: IdentityKey,
    view: ViewKey,
    spend: SpendKey,
}

impl Persona {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            identity: IdentityKey::generate(),
            view: ViewKey::generate(),
            spend: SpendKey::generate(),
        }
    }
    fn address(&self) -> PhantomAddress {
        PhantomAddress::new(self.view.public, self.spend.public)
    }
    fn signing(&self) -> PhantomSigningKey {
        PhantomSigningKey::from_bytes(self.identity.private)
    }
}

pub fn run() -> Result<()> {
    println!("{}", "═══ PHANTOMCHAT CRYPTO DEMO ═══".magenta().bold());
    println!("{}", "in-process showcase — no network, no disk".dimmed());

    h("IDENTITY GENERATION");
    let alice = Persona::new("alice");
    let bob = Persona::new("bob");
    let carol = Persona::new("carol");
    for p in [&alice, &bob, &carol] {
        ok(&format!("{} generated", p.name.yellow()));
        detail(
            "view pub",
            &format!("{}…", &hex::encode(p.view.public.to_bytes())[..16]),
        );
        detail(
            "spend pub",
            &format!("{}…", &hex::encode(p.spend.public.to_bytes())[..16]),
        );
    }

    h("SENDER-KEYS GROUP   alice + bob + carol");
    let roster = vec![alice.address(), bob.address(), carol.address()];
    let mut g_alice = PhantomGroup::new(roster.clone(), &alice.signing());
    let mut g_bob = PhantomGroup::new(roster.clone(), &bob.signing());
    let mut g_carol = PhantomGroup::new(roster, &carol.signing());
    // Mimic `phantom group join`: align everyone to creator's group_id.
    g_bob.group_id = g_alice.group_id;
    g_carol.group_id = g_alice.group_id;
    detail("group_id", &hex::encode(g_alice.group_id));

    let d_alice = g_alice.own_distribution(&alice.signing());
    let d_bob = g_bob.own_distribution(&bob.signing());
    let d_carol = g_carol.own_distribution(&carol.signing());
    g_bob.accept_distribution(d_alice.clone());
    g_carol.accept_distribution(d_alice);
    g_alice.accept_distribution(d_bob.clone());
    g_carol.accept_distribution(d_bob);
    g_alice.accept_distribution(d_carol.clone());
    g_bob.accept_distribution(d_carol);
    ok("sender-keys exchanged between all 3 members");

    let a_wire = g_alice.encrypt(&alice.signing(), b"hi team, demo time");
    let b_plain = g_bob.decrypt(&a_wire).map_err(|e| anyhow!("{e:?}"))?;
    let c_plain = g_carol.decrypt(&a_wire).map_err(|e| anyhow!("{e:?}"))?;
    ok(&format!(
        "{} encrypts {}-byte wire",
        "alice".yellow(),
        a_wire.len()
    ));
    ok(&format!(
        "{} + {} both decrypt:  {:?}",
        "bob".yellow(),
        "carol".yellow(),
        String::from_utf8_lossy(&b_plain)
    ));
    debug_assert_eq!(b_plain, c_plain);

    let b_wire = g_bob.encrypt(&bob.signing(), b"ack, all green");
    let a_plain = g_alice.decrypt(&b_wire).map_err(|e| anyhow!("{e:?}"))?;
    let c_plain = g_carol.decrypt(&b_wire).map_err(|e| anyhow!("{e:?}"))?;
    ok(&format!(
        "{} encrypts — {} + {} both decrypt:  {:?}",
        "bob".yellow(),
        "alice".yellow(),
        "carol".yellow(),
        String::from_utf8_lossy(&a_plain)
    ));
    debug_assert_eq!(a_plain, c_plain);

    let c_wire = g_carol.encrypt(&carol.signing(), b"all good my side too");
    let a_plain = g_alice.decrypt(&c_wire).map_err(|e| anyhow!("{e:?}"))?;
    let b_plain = g_bob.decrypt(&c_wire).map_err(|e| anyhow!("{e:?}"))?;
    ok(&format!(
        "{} encrypts — {} + {} both decrypt:  {:?}",
        "carol".yellow(),
        "alice".yellow(),
        "bob".yellow(),
        String::from_utf8_lossy(&a_plain)
    ));
    debug_assert_eq!(a_plain, b_plain);

    h("SIGNATURE ENFORCEMENT   forged message must be rejected");
    let mut tampered = a_wire.clone();
    // Flip a byte in the ciphertext area (after version+group_id+iteration+nonce).
    let flip_idx = 1 + 16 + 4 + 24 + 4 + 5;
    if flip_idx < tampered.len() {
        tampered[flip_idx] ^= 0xFF;
    }
    match g_bob.decrypt(&tampered) {
        Ok(_) => return Err(anyhow!("DEMO FAIL — tampered wire was accepted!")),
        Err(_) => ok("tampered wire rejected by receiver"),
    }

    println!(
        "\n{}   full group primitive round-trips cleanly. run `phantom selftest` for the 30-check audit.",
        "✓ DEMO PASSED".green().bold()
    );
    Ok(())
}
