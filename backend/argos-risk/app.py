"""Argos /api/risk — real scoring engine.

Wire contract (unchanged) consumed by the Argos mobile/desktop pre-send
risk check. Three layers:

1. ALLOWLIST — established tokens (stablecoins, wSOL, blue-chip SPL) are
   always clean. The Pylonyx memecoin score-engine is tuned for FRESH
   launches and (correctly for its purpose) scores even USDC as "amber"
   because LP-lock / deployer-history signals don't apply to it. Sending
   USDC must never warn the user, so established mints fast-path to clean.

2. UNKNOWN MINTS -> forward to the Pylonyx scan engine
   (http://127.0.0.1:3011/api/scan/<mint>) which runs GoPlus token-security
   + deployer-rug-history + LP-lock + holder-concentration. Pylonyx returns
   a SAFETY score (0..100, higher = safer); Argos risk = 100 - safety.
   Hard penalties (active mint/freeze authority etc.) surface as warnings.
   On any error/timeout -> amber (no-data), never a silent clean.

3. WALLETS -> blacklist lookup (known scam/drainer addresses in
   /opt/argos-risk/blacklist.json). Listed -> red. Otherwise clean. (Full
   smart-money classification is a heavier Pylonyx job; the blacklist is
   the high-signal, low-false-positive layer.)
"""
from __future__ import annotations

import json
import os
import urllib.request
from typing import List, Literal, Optional

from fastapi import FastAPI, Header, HTTPException
from pydantic import BaseModel, Field

app = FastAPI(title="Argos Risk API", version="1.0.0")

PYLONYX_SCAN_BASE = os.environ.get(
    "PYLONYX_SCAN_BASE", "http://127.0.0.1:3011/api/scan"
)
BLACKLIST_PATH = os.environ.get(
    "ARGOS_BLACKLIST_PATH", "/opt/argos-risk/blacklist.json"
)
SCAN_TIMEOUT_SEC = 6.0


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


# ── Layer 1: established-token allowlist (always clean) ────────────────────
ALLOWLIST = {
    "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v": "USDC",
    "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB": "USDT",
    "So11111111111111111111111111111111111111112": "Wrapped SOL",
    "JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN": "Jupiter (JUP)",
    "DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263": "Bonk (BONK)",
    "EKpQGSJtjMFqKZ9KQanSqYXRcF8fBopzLHYxdM65zcjm": "dogwifhat (WIF)",
    "jtojtomepa8beP8AuQc6eXt5FriJwfFMwQx2v2f9mCL": "Jito (JTO)",
    "HZ1JovNiVvGrGNiiYvEozEVgZ58xaU3RKwX8eACQBCt3": "Pyth (PYTH)",
    "mSoLzYCxHdYgdzU16g5QSh3i5K3z3KZK7ytfqcJm7So": "Marinade SOL (mSOL)",
    "7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs": "Wrapped ETH (Wormhole)",
    "9n4nbM75f5Ui33ZbPYXn59EwSgE8CGsHtAeTH5YFeJ9D": "Wrapped BTC (Wormhole)",
    "4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R": "Raydium (RAY)",
    "orcaEKTdK7LKz57vaAYr9QeNsVEPfiu6QeMU1kektZE": "Orca (ORCA)",
    "rndrizKT3MK1iimdxRdWabcF7Zg7AR5T4nud4EkHBof": "Render (RNDR)",
    "7GCihgDB8fe6KNjn2MYtkzZcRjQy3t9GHdC8uHYmW2hr": "POPCAT",
    "EKpQGSJtjMFqKZ9KQanSqYXRcF8fBopzLHYxdM65zcjm": "dogwifhat (WIF)",
}


def _score_known_clean(mint: str, name: str) -> RiskResult:
    return RiskResult(
        type="token",
        mint=mint,
        score=5,
        warnings=[],
        metadata={"name": name, "trusted": True, "layer": "allowlist"},
    )


# ── Layer 2: forward unknown mints to Pylonyx scan ─────────────────────────
def _pylonyx_scan(mint: str) -> Optional[dict]:
    url = f"{PYLONYX_SCAN_BASE}/{mint}"
    try:
        req = urllib.request.Request(url, headers={"Accept": "application/json"})
        with urllib.request.urlopen(req, timeout=SCAN_TIMEOUT_SEC) as resp:
            if resp.status != 200:
                return None
            return json.loads(resp.read().decode("utf-8"))
    except Exception:
        return None


def _score_token(mint: str) -> RiskResult:
    if mint in ALLOWLIST:
        return _score_known_clean(mint, ALLOWLIST[mint])

    scan = _pylonyx_scan(mint)
    if not scan or "score" not in scan:
        # No data -> amber, never silent-clean. The client surfaces a caution.
        return RiskResult(
            type="token",
            mint=mint,
            score=40,
            warnings=["no_risk_data_available"],
            metadata={"trusted": False, "layer": "no-data"},
        )

    score_obj = scan.get("score", {})
    safety = score_obj.get("totalScore")
    if not isinstance(safety, (int, float)):
        return RiskResult(
            type="token",
            mint=mint,
            score=40,
            warnings=["no_risk_data_available"],
            metadata={"trusted": False, "layer": "no-data"},
        )

    # Pylonyx safety (higher = safer) -> Argos risk (higher = riskier).
    risk = max(0, min(100, 100 - int(round(safety))))
    warnings: List[str] = []
    for hp in score_obj.get("hardPenalties", []) or []:
        # hardPenalties items can be strings or dicts depending on engine ver.
        if isinstance(hp, str):
            warnings.append(hp)
        elif isinstance(hp, dict):
            warnings.append(str(hp.get("reason") or hp.get("label") or "hard_penalty"))
    verdict = score_obj.get("verdict")  # gruen | gelb | rot
    if verdict == "rot":
        warnings.append("pylonyx_verdict_red")
    name = (scan.get("input", {}).get("metadata", {}) or {}).get("symbol")
    return RiskResult(
        type="token",
        mint=mint,
        score=risk,
        warnings=warnings,
        metadata={
            "trusted": risk < 30,
            "layer": "pylonyx-scan",
            "pylonyx_safety": int(round(safety)),
            "verdict": verdict,
            "name": name,
        },
    )


# ── Layer 3: wallet blacklist ──────────────────────────────────────────────
_blacklist_cache: Optional[set] = None


def _load_blacklist() -> set:
    global _blacklist_cache
    if _blacklist_cache is not None:
        return _blacklist_cache
    try:
        with open(BLACKLIST_PATH, "r", encoding="utf-8") as f:
            data = json.load(f)
        # Accept either a bare list or {"addresses": [...]}.
        addrs = data.get("addresses", data) if isinstance(data, dict) else data
        _blacklist_cache = {str(a).strip() for a in addrs}
    except Exception:
        _blacklist_cache = set()
    return _blacklist_cache


def _score_wallet(address: str) -> RiskResult:
    if address in _load_blacklist():
        return RiskResult(
            type="wallet",
            address=address,
            score=95,
            warnings=["address_on_scam_blacklist"],
            metadata={"trusted": False, "layer": "blacklist"},
        )
    return RiskResult(
        type="wallet",
        address=address,
        score=10,
        warnings=[],
        metadata={"trusted": True, "layer": "blacklist-clean"},
    )


@app.get("/")
def root():
    return {"service": "argos-risk", "version": "1.0.0", "status": "ok"}


@app.get("/health")
def health():
    return {"ok": True, "blacklist_size": len(_load_blacklist())}


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
    return CheckResponse(
        results=results,
        tier_used=tier,
        rate_limit_remaining=100 if tier == "free" else 10000,
    )
