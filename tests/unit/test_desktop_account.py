"""Desktop account API: real quota semantics, auth boundaries and refresh outcomes."""
import asyncio
from datetime import datetime, timezone, timedelta
from types import SimpleNamespace
from unittest.mock import AsyncMock

import httpx
from httpx import AsyncClient as RealAsyncClient
import pytest
from fastapi import FastAPI
from fastapi.testclient import TestClient

from kiro.auth import KiroAuthManager
from kiro.desktop_account import router, summarize_usage, DesktopAccountService


def usage(**over):
    """Build a provider-shaped usage response with no real account data."""
    item = {"resourceType": "CREDIT", "currentUsage": 139, "usageLimit": 2000, **over}
    return {"userInfo": {"email": "sample@example.com"}, "subscriptionInfo": {"subscriptionTitle": "KIRO PRO+"}, "usageBreakdownList": [item]}


def test_precision_and_remaining():
    value = summarize_usage(usage(currentUsageWithPrecision=139.25), 100)
    assert value["used"] == 139.25
    assert value["remaining"] == 1860.75
    assert value["usagePercent"] == pytest.approx(6.9625)


def test_trial_bonus_and_paid_overage_are_separate():
    data = usage(freeTrialInfo={"freeTrialStatus": "ACTIVE", "usageLimit": 500, "currentUsage": 20}, bonuses=[
        {"status": "ACTIVE", "expiresAt": 200, "usageLimit": 100, "currentUsage": 5},
        {"status": "ACTIVE", "expiresAt": 50, "usageLimit": 900, "currentUsage": 0},
        {"status": "EXPIRED", "usageLimit": 900}], currentOveragesWithPrecision=1.25, overageCap=10000)
    data["overageConfiguration"] = {"overageStatus": "ENABLED"}
    result = summarize_usage(data, 100)
    assert (result["used"], result["total"], result["remaining"]) == (164, 2600, 2436)
    assert result["overageUsed"] == 1.25 and result["overageEnabled"]


@pytest.mark.parametrize("items", [[], [{"resourceType": "OTHER", "usageLimit": 10}], [{"resourceType": "CREDIT"}]])
def test_missing_credit_is_unknown(items):
    result = summarize_usage({"usageBreakdownList": items}, 100)
    assert result["total"] is None and result["remaining"] is None


@pytest.mark.parametrize("used,total", [(3, 0), (3000, 2000)])
def test_zero_and_exhausted_quota(used, total):
    result = summarize_usage(usage(currentUsage=used, usageLimit=total), 100)
    assert result["remaining"] == 0
    assert result["usagePercent"] == (None if total == 0 else 150)


def test_credit_entry_need_not_be_first_and_bad_numbers_stay_unknown():
    data = usage(currentUsageWithPrecision=float('nan'), currentUsage=3)
    data['usageBreakdownList'].insert(0, {'resourceType': 'OTHER', 'currentUsage': 99, 'usageLimit': 100})
    assert summarize_usage(data, 100)['used'] == 3
    assert summarize_usage(usage(usageLimit=-1), 100)['total'] is None


def auth():
    """An in-memory auth owner, isolated from the user's credentials."""
    a = KiroAuthManager(refresh_token='fake-refresh')
    a._access_token = 'fake-access'
    a._expires_at = datetime.now(timezone.utc) + timedelta(hours=1)
    return a


@pytest.mark.asyncio
async def test_usage_uses_runtime_owner_and_never_returns_secrets():
    a = auth()
    requests = []
    def respond(req):
        requests.append(req)
        return httpx.Response(200, json=usage())
    async with RealAsyncClient(transport=httpx.MockTransport(respond)) as client:
        result = await DesktopAccountService(a, client).snapshot()
    assert result['email'] == 'sample@example.com'
    assert result['remaining'] == 1861
    assert 'fake-' not in str(result)
    assert requests[0].headers['authorization'] == 'Bearer fake-access'
    assert requests[0].url.host == 'management.us-east-1.kiro.dev'
    assert 'profileArn' not in requests[0].url.params


@pytest.mark.asyncio
async def test_refresh_success_is_distinct_from_usage_failure():
    a = auth()
    a.refresh_for_desktop = AsyncMock()
    async with RealAsyncClient(transport=httpx.MockTransport(lambda _: httpx.Response(403, text='fake-secret'))) as client:
        result = await DesktopAccountService(a, client).snapshot(refresh=True)
    a.refresh_for_desktop.assert_awaited_once()
    assert result['tokenRefreshedAt']
    assert result['usageError'] and result['total'] is None
    assert 'fake-secret' not in str(result)


@pytest.mark.asyncio
async def test_missing_profile_resolves_current_account_instead_of_cached_profile():
    a = auth()
    a._profile_arn = 'stale-profile'
    requests = []
    def respond(req):
        requests.append(req)
        if len(requests) == 1:
            return httpx.Response(400, json={'message': 'profileArn is required for this request.'})
        if len(requests) == 2:
            return httpx.Response(200, json={'profiles': [{'arn': 'current-profile'}]})
        assert req.url.params['profileArn'] == 'current-profile'
        return httpx.Response(200, json=usage())
    async with RealAsyncClient(transport=httpx.MockTransport(respond)) as client:
        result = await DesktopAccountService(a, client).snapshot()
    assert result['email'] == 'sample@example.com'
    assert all('stale-profile' not in str(r.url) for r in requests)


def test_routes_disabled_by_default_and_require_key_and_single_account(monkeypatch):
    import kiro.desktop_account as module
    app = FastAPI(); app.include_router(router)
    app.state.account_manager = SimpleNamespace(get_first_account=lambda: SimpleNamespace(auth_manager=auth()))
    app.state.account_system = False
    client = TestClient(app)
    monkeypatch.setattr(module, 'DESKTOP_API_ENABLED', False)
    assert client.get('/desktop/account').status_code == 404
    monkeypatch.setattr(module, 'DESKTOP_API_ENABLED', True)
    assert client.get('/desktop/account').status_code == 401
    monkeypatch.setattr('kiro.routes_openai.PROXY_API_KEY', 'test-key')
    app.state.account_system = True
    assert client.get('/desktop/account', headers={'Authorization': 'Bearer test-key'}).status_code == 409


def sqlite_auth(tmp_path):
    """Create an isolated CLI database with refreshable credentials."""
    import sqlite3, json
    p = tmp_path / 'cli.sqlite'
    data = {'access_token': 'old-access', 'refresh_token': 'old-refresh', 'expires_at': '2099-01-01T00:00:00Z', 'provider': 'Github', 'unrelated': 'keep'}
    with sqlite3.connect(p) as conn:
        conn.execute('CREATE TABLE auth_kv (key TEXT PRIMARY KEY, value TEXT)')
        conn.execute('INSERT INTO auth_kv VALUES (?, ?)', ('kirocli:social:token', json.dumps(data)))
    return KiroAuthManager(sqlite_db=str(p)), p


@pytest.mark.asyncio
async def test_manual_refresh_persists_rotated_tokens_and_preserves_fields(tmp_path, monkeypatch):
    import sqlite3, json
    a, p = sqlite_auth(tmp_path)
    monkeypatch.setattr('kiro.auth.SQLITE_READONLY', False)
    async def refresh():
        a._access_token, a._refresh_token = 'rotated-access', 'rotated-refresh'
        a._save_credentials_to_sqlite()
    a._refresh_token_request = refresh
    await a.refresh_for_desktop()
    a.assert_desktop_source_current()
    with sqlite3.connect(p) as conn:
        data = json.loads(conn.execute('SELECT value FROM auth_kv').fetchone()[0])
    assert data['access_token'] == 'rotated-access' and data['unrelated'] == 'keep'
    assert a.desktop_metadata()['provider'] == 'GitHub'


@pytest.mark.asyncio
@pytest.mark.parametrize('during_refresh', [True, False])
async def test_switched_cli_account_is_never_overwritten(tmp_path, monkeypatch, during_refresh):
    import sqlite3, json
    from kiro.auth import CredentialSourceChanged
    a, p = sqlite_auth(tmp_path)
    monkeypatch.setattr('kiro.auth.SQLITE_READONLY', False)
    def switch():
        with sqlite3.connect(p) as conn:
            conn.execute('UPDATE auth_kv SET value = ?', (json.dumps({'access_token': 'other-account', 'refresh_token': 'other-refresh'}),))
    async def refresh():
        switch()
        a._access_token = 'new-old-account-token'
        a._save_credentials_to_sqlite()
    a._refresh_token_request = AsyncMock(side_effect=refresh)
    if not during_refresh:
        switch()
    with pytest.raises(CredentialSourceChanged):
        await a.refresh_for_desktop()
    if not during_refresh:
        a._refresh_token_request.assert_not_awaited()
    with sqlite3.connect(p) as conn:
        data = json.loads(conn.execute('SELECT value FROM auth_kv').fetchone()[0])
    assert data['access_token'] == 'other-account'


@pytest.mark.asyncio
async def test_readonly_mode_disables_manual_refresh(tmp_path, monkeypatch):
    a, _ = sqlite_auth(tmp_path)
    monkeypatch.setattr('kiro.auth.SQLITE_READONLY', True)
    a._refresh_token_request = AsyncMock()
    assert not a.desktop_metadata()['canRefreshToken']
    with pytest.raises(ValueError):
        await a.refresh_for_desktop()
    a._refresh_token_request.assert_not_awaited()


@pytest.mark.asyncio
@pytest.mark.parametrize('status', [401, 403, 429, 500])
async def test_quota_failure_is_safe_and_never_a_zero_balance(status):
    async with RealAsyncClient(transport=httpx.MockTransport(lambda _: httpx.Response(status, text='secret-credential'))) as client:
        result = await DesktopAccountService(auth(), client).snapshot()
    assert result['usageError'] and result['remaining'] is None
    assert 'secret-credential' not in str(result)


@pytest.mark.asyncio
async def test_account_requests_are_serialized_on_one_runtime_owner():
    a = auth()
    entered, release = asyncio.Event(), asyncio.Event()
    async with RealAsyncClient(transport=httpx.MockTransport(lambda _: httpx.Response(200, json=usage()))) as client:
        service = DesktopAccountService(a, client)
        calls = []
        async def fetch(token):
            calls.append(token)
            entered.set()
            await release.wait()
            return usage()
        service._usage = fetch
        one = asyncio.create_task(service.snapshot())
        await entered.wait()
        two = asyncio.create_task(service.snapshot())
        await asyncio.sleep(0)
        assert len(calls) == 1
        release.set()
        await asyncio.gather(one, two)
        assert len(calls) == 2


@pytest.mark.asyncio
async def test_token_refresh_route_returns_only_public_metadata(monkeypatch):
    import kiro.desktop_account as module
    a = auth()
    async def refresh():
        a._access_token = 'rotated-private-access'
        a._expires_at = datetime.now(timezone.utc) + timedelta(hours=2)
    a.refresh_for_desktop = AsyncMock(side_effect=refresh)
    app = FastAPI(); app.include_router(router)
    app.state.account_manager = SimpleNamespace(get_first_account=lambda: SimpleNamespace(auth_manager=a))
    app.state.account_system = False
    monkeypatch.setattr(module, 'DESKTOP_API_ENABLED', True)
    monkeypatch.setattr('kiro.routes_openai.PROXY_API_KEY', 'local-test-key')
    async with RealAsyncClient(transport=httpx.MockTransport(lambda req: httpx.Response(200, json=usage()))) as upstream:
        app.state.http_client = upstream
        async with RealAsyncClient(transport=httpx.ASGITransport(app=app), base_url='http://test') as client:
            assert (await client.post('/desktop/account/refresh-token')).status_code == 401
            response = await client.post('/desktop/account/refresh-token', headers={'Authorization': 'Bearer local-test-key'})
    assert response.status_code == 200
    assert response.json()['tokenRefreshedAt']
    assert response.json()['email'] == 'sample@example.com'
    assert 'private-access' not in response.text and 'fake-refresh' not in response.text
    a.refresh_for_desktop.assert_awaited_once()


@pytest.mark.asyncio
async def test_cli_changes_and_provider_token_failure_are_sanitized(monkeypatch):
    import kiro.desktop_account as module
    from kiro.auth import CredentialSourceChanged
    a = auth()
    a.refresh_for_desktop = AsyncMock(side_effect=CredentialSourceChanged('private-source'))
    app = FastAPI(); app.include_router(router)
    app.state.account_manager = SimpleNamespace(get_first_account=lambda: SimpleNamespace(auth_manager=a))
    app.state.account_system = False
    monkeypatch.setattr(module, 'DESKTOP_API_ENABLED', True)
    monkeypatch.setattr('kiro.routes_openai.PROXY_API_KEY', 'local-test-key')
    async with RealAsyncClient() as upstream:
        app.state.http_client = upstream
        async with RealAsyncClient(transport=httpx.ASGITransport(app=app), base_url='http://test') as client:
            response = await client.post('/desktop/account/refresh-token', headers={'Authorization': 'Bearer local-test-key'})
            assert response.status_code == 409 and 'private-source' not in response.text
            a.refresh_for_desktop.side_effect = ValueError('private-provider-response')
            response = await client.post('/desktop/account/refresh-token', headers={'Authorization': 'Bearer local-test-key'})
            assert response.status_code == 502 and 'private-provider-response' not in response.text


@pytest.mark.asyncio
async def test_manual_oidc_failure_does_not_reload_and_refresh_a_new_account(tmp_path, monkeypatch):
    import sqlite3, json
    from kiro.auth import AuthType, CredentialSourceChanged
    a, p = sqlite_auth(tmp_path)
    monkeypatch.setattr('kiro.auth.SQLITE_READONLY', False)
    a._auth_type = AuthType.AWS_SSO_OIDC
    async def switched_failure():
        with sqlite3.connect(p) as conn:
            conn.execute('UPDATE auth_kv SET value = ?', (json.dumps({'access_token': 'other-account', 'refresh_token': 'other-refresh'}),))
        request = httpx.Request('POST', 'https://example.invalid/token')
        raise httpx.HTTPStatusError('invalid request', request=request, response=httpx.Response(400, request=request))
    a._do_aws_sso_oidc_refresh = AsyncMock(side_effect=switched_failure)
    with pytest.raises(CredentialSourceChanged):
        await a.refresh_for_desktop()
    a._do_aws_sso_oidc_refresh.assert_awaited_once()
    assert a._access_token == 'old-access'
