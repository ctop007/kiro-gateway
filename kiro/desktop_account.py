"""Opt-in desktop account controls using the gateway's actual credential owner."""
import asyncio
import math
from datetime import datetime, timezone
from typing import Any, Optional

import httpx
from fastapi import APIRouter, Depends, HTTPException, Request

from kiro.auth import KiroAuthManager, CredentialSourceChanged
from kiro.config import DESKTOP_API_ENABLED
from kiro.routes_openai import verify_api_key
from kiro.utils import get_kiro_headers

router = APIRouter(prefix="/desktop", include_in_schema=False)


def amount(data: dict, key: str) -> Optional[float]:
    """Read a finite nonnegative quantity, preferring the precise provider field."""
    for field in (key + "WithPrecision", key):
        value = data.get(field)
        if isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value) and value >= 0:
            return float(value)
    return None


def timestamp(value: Any) -> Optional[str]:
    """Convert a provider epoch timestamp, treating missing/invalid values as unknown."""
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        try:
            return datetime.fromtimestamp(value, timezone.utc).isoformat()
        except (ValueError, OverflowError, OSError):
            pass
    return None


def summarize_usage(data: dict, now: float) -> dict:
    """Normalize account credits without counting paid overage as free remaining quota.

    Args:
        data: Provider getUsageLimits JSON.
        now: Current epoch seconds, used to exclude expired bonus credits.

    Returns:
        Only account display fields; no tokens or raw provider diagnostics.
    """
    entries = data.get("usageBreakdownList") or []
    item = next((i for i in entries if isinstance(i, dict) and i.get("resourceType") == "CREDIT"), {})
    used, total = amount(item, "currentUsage"), amount(item, "usageLimit")
    extras = []
    trial = item.get("freeTrialInfo") or {}
    if trial.get("freeTrialStatus") == "ACTIVE":
        extras.append(trial)
    for bonus in item.get("bonuses") or []:
        expiry = bonus.get("expiresAt")
        if bonus.get("status") == "ACTIVE" and (expiry is None or (isinstance(expiry, (int, float)) and expiry > now)):
            extras.append(bonus)
    for extra in extras:
        extra_used, extra_total = amount(extra, "currentUsage"), amount(extra, "usageLimit")
        # Missing numeric fields must never silently inflate remaining credits.
        used = used + extra_used if used is not None and extra_used is not None else None
        total = total + extra_total if total is not None and extra_total is not None else None
    known = used is not None and total is not None
    return {
        "email": (data.get("userInfo") or {}).get("email"),
        "subscription": (data.get("subscriptionInfo") or {}).get("subscriptionTitle"),
        "used": used, "total": total,
        "remaining": max(0.0, total - used) if known else None,
        "usagePercent": used / total * 100 if known and total > 0 else None,
        "resetAt": timestamp(item.get("nextDateReset") or data.get("nextDateReset")),
        "overageEnabled": (data.get("overageConfiguration") or {}).get("overageStatus") == "ENABLED",
        "overageUsed": amount(item, "currentOverages"),
    }


class DesktopAccountService:
    """Query usage and refresh the exact auth manager used by generation requests."""

    def __init__(self, auth: KiroAuthManager, client: httpx.AsyncClient) -> None:
        self.auth = auth
        self.client = client
        self.lock = asyncio.Lock()

    async def _usage(self, token: str) -> dict:
        """Query account identity; resolve required profiles from the same token."""
        headers = get_kiro_headers(self.auth, token)
        headers.pop("x-amz-target", None)
        headers["Accept"] = "application/json"
        params = {"isEmailRequired": "true", "origin": "AI_EDITOR", "resourceType": "AGENTIC_REQUEST"}
        base = self.auth.management_host
        response = await self.client.get(f"{base}/getUsageLimits", headers=headers, params=params, timeout=15)
        if response.status_code == 400 and "profile" in response.text.lower():
            # Never reuse the CLI's potentially stale cached profile for the account card.
            profiles = await self.client.post(
                base, headers={**headers, "x-amz-target": "KiroControlPlaneBearerService.ListAvailableProfiles"},
                json={}, timeout=15,
            )
            profiles.raise_for_status()
            candidates = profiles.json().get("profiles", [])
            arns = [p.get("arn") for p in candidates if isinstance(p, dict) and p.get("arn")]
            if len(arns) != 1:
                raise ValueError("Cannot select an unambiguous account profile")
            params["profileArn"] = arns[0]
            response = await self.client.get(f"{base}/getUsageLimits", headers=headers, params=params, timeout=15)
        response.raise_for_status()
        data = response.json()
        if not isinstance(data, dict):
            raise ValueError("Invalid usage response")
        return data

    async def snapshot(self, refresh: bool = False) -> dict:
        """Return safe account metadata; quota errors cannot undo a successful refresh.

        Args:
            refresh: Explicitly renew the current credentials before querying usage.

        Returns:
            A public snapshot; token material never leaves this service.
        """
        async with self.lock:
            self.auth.assert_desktop_source_current()
            refreshed = None
            if refresh:
                await self.auth.refresh_for_desktop()
                refreshed = datetime.now(timezone.utc).isoformat()
            token = await self.auth.get_access_token()
            result = summarize_usage({}, 0)
            error = None
            try:
                result = summarize_usage(await asyncio.wait_for(self._usage(token), timeout=30), datetime.now(timezone.utc).timestamp())
                if result["total"] is None or result["used"] is None:
                    error = "未获取到完整额度，稍后刷新额度重试"
            except httpx.HTTPStatusError as exc:
                error = ("额度查询鉴权失败，请刷新 Token 后重试" if exc.response.status_code in (401, 403)
                         else "额度查询失败，稍后刷新额度重试")
            except (httpx.RequestError, ValueError, TypeError, AttributeError, asyncio.TimeoutError):
                error = "暂时无法获取账号额度，请检查网络后重试"
            return {
                **result, **self.auth.desktop_metadata(), "usageError": error,
                "checkedAt": datetime.now(timezone.utc).isoformat(), "tokenRefreshedAt": refreshed,
            }


async def desktop_access(request: Request) -> None:
    """Hide controls unless explicitly enabled, then verify the gateway access key."""
    if not DESKTOP_API_ENABLED:
        raise HTTPException(404, "Not found")
    await verify_api_key(request.headers.get("authorization"), request.headers.get("x-api-key"))
    if getattr(request.app.state, "account_system", False):
        raise HTTPException(409, "Desktop account controls require single-account mode")


async def account_snapshot(request: Request, refresh: bool) -> dict:
    """Bind one service to the runtime owner, sanitizing every failure at the boundary."""
    auth = request.app.state.account_manager.get_first_account().auth_manager
    service = getattr(request.app.state, "desktop_account_service", None)
    if service is None or service.auth is not auth:
        service = DesktopAccountService(auth, request.app.state.http_client)
        request.app.state.desktop_account_service = service
    try:
        return await asyncio.wait_for(service.snapshot(refresh), timeout=70)
    except CredentialSourceChanged:
        raise HTTPException(409, "CLI credentials changed; restart the gateway") from None
    except (httpx.HTTPError, ValueError, OSError, asyncio.TimeoutError):
        # Provider bodies can contain credentials. Never return them through IPC.
        raise HTTPException(502, "Account operation failed") from None


@router.get("/account", dependencies=[Depends(desktop_access)])
async def get_account(request: Request) -> dict:
    """Read the current gateway account and fresh usage."""
    return await account_snapshot(request, False)


@router.post("/account/refresh-token", dependencies=[Depends(desktop_access)])
async def refresh_account_token(request: Request) -> dict:
    """Refresh the active token without exposing it to the desktop frontend."""
    return await account_snapshot(request, True)
