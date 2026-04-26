# Self-Hosted PhantomChat Relay

> Run your own Nostr relay so PhantomChat envelopes never touch a third-party
> service. Aimed at organisations (Kanzleien, Steuerberater, Krankenhäuser,
> Behörden) that need full data-sovereignty for their internal comms.

Audience: an ops engineer who can already operate Docker / nginx / Let's
Encrypt on a Linux VPS. The whole walkthrough is ~ 30 minutes from a fresh
VM to "PhantomChat clients can talk to it via TLS".

---

## Why self-host?

PhantomChat is metadata-blind by design — relays only ever see opaque
sealed-sender envelopes that look identical to cover-traffic. So why bother
self-hosting if the public relays (`relay.damus.io`, `nos.lol`,
`relay.snort.social`) literally cannot read your messages?

Four reasons:

1. **Full data-sovereignty.** No third-party Nostr relay sees your envelopes
   at all — not even encrypted ones, not even the timing pattern. The
   off-cluster TCP-layer metadata (source IP, packet sizes, connection
   uptime) stays inside infrastructure you control.
2. **Cleaner DSGVO + § 203 StGB story.** No Auftragsverarbeitungs-Vertrag
   (AVV) needed for relay traffic, because *you operate the relay yourself*.
   For Berufsgeheimnisträger (lawyers, doctors, tax advisors) this collapses
   one of the trickiest contractual hot-spots in the public-relay model.
3. **Independence from operator policy.** Damus / nos.lol can change their
   write policy, rate-limits, or shut down entirely. A self-hosted relay
   can't get a "your account has been suspended" email.
4. **LAN-local latency.** Sub-millisecond round-trip if the relay sits in
   the same office network, vs. 30–50 ms to a public relay. Group-chat
   commits feel snappier; cover-traffic is cheaper.

---

## Pick your relay implementation

Any NIP-01-compliant relay works. The four most production-ready open
implementations:

| Relay              | Lang | Storage          | Best for                                       |
| ------------------ | ---- | ---------------- | ---------------------------------------------- |
| `strfry`           | C++  | LMDB             | High-performance, large orgs (10k+ users)      |
| `nostr-rs-relay`   | Rust | SQLite           | Medium orgs, easy ops, single binary           |
| `nostream`         | TS   | Postgres         | Already-Postgres ops, plugin ecosystem         |
| `khatru`           | Go   | BoltDB / Postgres| Customisable filtering / write-policy plugins  |

For most orgs we recommend **`strfry`** — it's the fastest of the four, has
the smallest operational surface (one C++ binary, one config file, LMDB on
disk, no DB to babysit), and the LMDB store survives unclean shutdowns
gracefully.

The rest of this doc is a `strfry`-on-Docker quick-start.

---

## Quick start: `strfry` on Docker

Prerequisites: Linux VPS with a public DNS A-record pointing at it
(`relay.your-org.de`), Docker + docker-compose installed, ports 80 + 443
open in the firewall.

### 1. `docker-compose.yml`

Drop this in `/srv/phantomchat-relay/docker-compose.yml`:

```yaml
services:
  strfry:
    image: pivorian/strfry:latest
    container_name: phantomchat-relay
    restart: unless-stopped
    ports:
      - "127.0.0.1:7777:7777"  # bound to localhost; nginx fronts TLS
    volumes:
      - ./strfry-db:/app/strfry-db
      - ./strfry.conf:/etc/strfry.conf:ro
    environment:
      - STRFRY_DB=/app/strfry-db
```

Note: we bind `7777` to `127.0.0.1` rather than `0.0.0.0`. The plaintext
WebSocket port should never be reachable from the internet directly — only
through nginx + TLS. The `ports:` mapping in the spec ("443:7777") is a
shorthand for "this is the port nginx will proxy to"; in practice you want
the binding above so a misconfigured firewall can't accidentally expose
plaintext WS.

### 2. `strfry.conf`

Drop this next to the compose file as `/srv/phantomchat-relay/strfry.conf`:

```text
db = "/app/strfry-db"

relay {
    bind = "0.0.0.0"
    port = 7777

    info {
        name = "PhantomChat-Org-Relay"
        description = "Internal relay for ORG-NAME — not for public use"
        pubkey = ""
        contact = "ops@your-org.de"
    }

    # PhantomChat envelopes are indistinguishable from cover-traffic at the
    # relay layer, so a write-policy plugin would have nothing meaningful to
    # filter on. Leave it empty.
    writePolicy {
        plugin = ""
    }

    # Per-IP rate limit. 50 msg/s/IP is plenty for normal chat + cover; bump
    # if you have a genuine power-user cluster behind a single NAT.
    maxClientMsgPerSec = 50

    # Hard cap on a single envelope. PhantomChat envelopes max out around
    # ~ 8 KiB even with PQXDH + padding, so 64 KiB is a comfortable ceiling
    # that also keeps LMDB growth predictable.
    maxEventBytes = 65536
}
```

### 3. nginx in front of it (TLS termination + WebSocket upgrade)

Add `/etc/nginx/sites-available/relay.your-org.de`:

```nginx
server {
    listen 443 ssl http2;
    server_name relay.your-org.de;

    ssl_certificate     /etc/letsencrypt/live/relay.your-org.de/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/relay.your-org.de/privkey.pem;

    location / {
        proxy_pass http://localhost:7777;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_read_timeout 86400s;  # WebSockets sit idle for hours
    }

    access_log /var/log/nginx/relay.your-org.de.access.log;
    error_log  /var/log/nginx/relay.your-org.de.error.log;
}

server {
    listen 80;
    server_name relay.your-org.de;
    return 301 https://$host$request_uri;
}
```

`ln -s` it into `sites-enabled/` and reload nginx (`nginx -t && systemctl
reload nginx`).

### 4. Let's Encrypt cert

```sh
certbot --nginx -d relay.your-org.de
```

Certbot rewrites the vhost to wire its cert paths in. The vhost above
already references the standard Let's-Encrypt path so the rewrite is a
no-op — certbot just installs the cert.

### 5. Bring up the relay

```sh
cd /srv/phantomchat-relay
docker compose up -d
```

Smoke-test with `wscat` from anywhere on the internet:

```sh
wscat -c wss://relay.your-org.de
> ["REQ","test",{"limit":1}]
< ["EOSE","test"]
```

If you see `EOSE` you have a working relay.

---

## Configure PhantomChat clients to use your relay

### Per-client (manual)

In each client: `Settings → Relays`, then either:

- **Hybrid**: `+ add relay` → `wss://relay.your-org.de`. Keep the public
  defaults so you have fallback if your relay is down.
- **Closed-loop** (full sovereignty): `+ add relay` →
  `wss://relay.your-org.de`, then `remove` the public defaults
  (`relay.damus.io`, `nos.lol`, `relay.snort.social`). This gives you a
  fully air-gapped-from-public-Nostr deployment.

### Org-wide bootstrap (Wave 7A mDNS-LAN-discovery flow)

If you ship preconfigured installers, edit the `bootstrap.json` baked into
your MSI / DMG so new installs come up pointing at your relay only:

```json
{
  "default_relays": ["wss://relay.your-org.de"]
}
```

The onboarding wizard's relay step will pre-select your relay and skip the
public-defaults block entirely.

---

## Operational notes

- **LMDB grows monotonically.** Even with `maxEventBytes = 65536` capping
  individual records, the database file only ever grows. Plan a periodic
  compaction:

  ```sh
  docker exec phantomchat-relay strfry export > /tmp/dump.jsonl
  docker exec phantomchat-relay strfry import < /tmp/dump.jsonl
  ```

  Run nightly via cron during a low-traffic window if your retention is
  rolling, or quarterly if you keep everything.

- **Backup.** `tar` the `strfry-db/` directory nightly, encrypt with `age`
  or `gpg`, push offsite (S3-compatible storage works fine):

  ```sh
  systemctl stop docker-compose@phantomchat-relay  # or `docker compose stop`
  tar czf - strfry-db/ | age -r ${OFFSITE_AGE_PUBKEY} > backup-$(date +%F).tar.gz.age
  systemctl start docker-compose@phantomchat-relay
  ```

  Restore is `tar xzf` into the same path before `docker compose up`.

- **Monitoring.** `strfry` exposes a Prometheus scrape endpoint on the
  internal port `:7778/metrics`. Add a `prometheus` container to the
  compose file and point it at `http://strfry:7778/metrics` — gives you
  per-client connection counts, event-rate, LMDB size, etc.

- **Log retention.** PhantomChat envelopes look identical to cover-traffic,
  so the only meaningful info in `relay.your-org.de.access.log` is
  TCP-level metadata (timestamps, source IP, byte counts). Keep nginx
  access logs **30 days max** to stay clean of DSGVO retention concerns;
  rotate via `logrotate` (Debian's default config is fine — just shorten
  `rotate 52` to `rotate 4` weekly).

---

## Federation — talking to another org's relay

Federation between org-relays is **not yet supported** as a first-class
PhantomChat feature. A future doc will cover NIP-65 / outbox-model
federation patterns.

**Workaround today:** both orgs configure each other's relay URLs in their
respective relay-lists. A 3-relay pool of `[wss://relay.org-a.de,
wss://relay.org-b.de, wss://nos.lol]` (the public one as fallback) gives
both sides redundancy without either being the single point of failure.
The `phantomchat_relays::make_multi_relay` adapter dedupes envelopes via a
4096-entry LRU on event-ID, so seeing the same envelope on multiple relays
costs you essentially nothing.

---

## Crash-Reporting endpoint (related infra)

If you self-host the relay, you may also want to point the (opt-in)
crash-report uploader at your own collection endpoint instead of
`updates.dc-infosec.de`. See `desktop/README.md` → *Crash Reporting* for
the client-side flow. Server-side, the endpoint is just a POST handler
that accepts a single JSON object and appends it to a file — any tiny
Python / Go / nginx-Lua script will do.
