#!/bin/bash
# Deploy argos-risk API service on hostinger:
#  - FastAPI stub on 127.0.0.1:7448
#  - nginx vhost for argos.dc-infosec.de + Letsencrypt
#  - systemd service argos-risk.service (Restart=always)
set -e

VENV=/opt/argos-risk/venv
SRC=/opt/argos-risk/app.py
SERVICE=/etc/systemd/system/argos-risk.service
NGINX_CONF=/etc/nginx/sites-available/argos.dc-infosec.de

echo "[1] python deps"
sudo mkdir -p /opt/argos-risk
if [ ! -d "$VENV" ]; then
  sudo python3 -m venv "$VENV"
fi
sudo "$VENV/bin/pip" install --quiet fastapi uvicorn 'pydantic>=2'

echo "[2] write stub app"
sudo tee "$SRC" >/dev/null << 'PYEOF'
"""Argos /api/risk stub.

This is the wire contract the Argos app speaks to. It returns deterministic
placeholder scores so the mobile + desktop client integration can be wired
end-to-end before the real Pylonyx-backed scoring engine lands.

Real scoring will be a thin shim that forwards the same payload into the
existing Pylonyx Next.js stack on port 3011.
"""
from __future__ import annotations

from typing import List, Literal, Optional

from fastapi import FastAPI, Header, HTTPException
from pydantic import BaseModel, Field

app = FastAPI(title="Argos Risk API", version="0.1.0")


class CheckItem(BaseModel):
    type: Literal["token", "wallet"]
    mint: Optional[str] = None
    address: Optional[str] = None


class CheckRequest(BaseModel):
    checks: List[CheckItem] = Field(min_length=1, max_length=16)


class RiskResult(BaseModel):
    type: Literal["token", "wallet"]
    mint: Optional[str] = None
    address: Optional[str] = None
    score: int = Field(ge=0, le=100)
    warnings: List[str]
    metadata: dict


class CheckResponse(BaseModel):
    results: List[RiskResult]
    tier_used: Literal["free", "vip"]
    rate_limit_remaining: int


# Hard-coded "known-good" tokens so the stub returns sensible defaults
# while the real scoring engine is wired up. USDC + USDT + SOL = clean.
KNOWN_CLEAN = {
    "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v": "USDC",
    "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB": "USDT",
    "So11111111111111111111111111111111111111112": "Wrapped SOL",
}


def _score_token(mint: str) -> RiskResult:
    if mint in KNOWN_CLEAN:
        return RiskResult(
            type="token", mint=mint, score=5, warnings=[],
            metadata={"name": KNOWN_CLEAN[mint], "trusted": True},
        )
    # Unknown token — return amber + a "needs review" warning so the UI
    # surfaces a non-aggressive caution. Real engine will replace this.
    return RiskResult(
        type="token", mint=mint, score=35,
        warnings=["unknown_token_no_pylonyx_data_yet"],
        metadata={"trusted": False},
    )


def _score_wallet(address: str) -> RiskResult:
    # Stub: all wallets return clean. Real engine will check against
    # the Pylonyx smart-money / blacklist tables.
    return RiskResult(
        type="wallet", address=address, score=10,
        warnings=[],
        metadata={"checked_at_layer": "stub"},
    )


@app.get("/")
def root():
    return {"service": "argos-risk", "version": "0.1.0", "status": "ok"}


@app.get("/health")
def health():
    return {"ok": True}


@app.post("/api/risk", response_model=CheckResponse)
def risk(req: CheckRequest, x_argos_tier: Optional[str] = Header(default="free")):
    tier = "vip" if x_argos_tier == "vip" else "free"
    results: List[RiskResult] = []
    for item in req.checks:
        if item.type == "token":
            if not item.mint:
                raise HTTPException(400, "token check requires mint")
            results.append(_score_token(item.mint))
        else:
            if not item.address:
                raise HTTPException(400, "wallet check requires address")
            results.append(_score_wallet(item.address))
    # Rate-limit is faked in the stub. The real engine ties this to a
    # Pylonyx-Account-API key via the X-Argos-Tier header.
    return CheckResponse(
        results=results,
        tier_used=tier,
        rate_limit_remaining=100 if tier == "free" else 10000,
    )
PYEOF

echo "[3] systemd service"
sudo tee "$SERVICE" >/dev/null << 'UNITEOF'
[Unit]
Description=Argos Risk API (FastAPI)
After=network.target

[Service]
ExecStart=/opt/argos-risk/venv/bin/uvicorn app:app --host 127.0.0.1 --port 7448
WorkingDirectory=/opt/argos-risk
Restart=always
RestartSec=3
User=www-data
Group=www-data

[Install]
WantedBy=multi-user.target
UNITEOF

sudo chown -R www-data:www-data /opt/argos-risk
sudo systemctl daemon-reload
sudo systemctl enable --now argos-risk.service
sleep 2
echo "[3] active: $(sudo systemctl is-active argos-risk.service)"

echo "[4] nginx vhost"
sudo tee "$NGINX_CONF" >/dev/null << 'NGINXEOF'
server {
    listen 80;
    server_name argos.dc-infosec.de;
    location / {
        proxy_pass http://127.0.0.1:7448;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
NGINXEOF
sudo ln -sf "$NGINX_CONF" /etc/nginx/sites-enabled/
sudo nginx -t && sudo systemctl reload nginx

echo "[5] letsencrypt cert"
sudo certbot --nginx -d argos.dc-infosec.de --non-interactive --agree-tos -m admin@dc-infosec.de --redirect 2>&1 | tail -5

echo "[6] verify"
curl -sS https://argos.dc-infosec.de/health
echo
curl -sS -X POST https://argos.dc-infosec.de/api/risk \
  -H 'content-type: application/json' \
  -d '{"checks":[{"type":"token","mint":"EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"}]}'
echo
echo "ARGOS_BACKEND_DONE"
