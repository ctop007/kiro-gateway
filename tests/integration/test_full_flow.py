# -*- coding: utf-8 -*-

"""
Integration tests for complete end-to-end flow.
Checks interaction of all system components.
"""

import pytest
import json
from unittest.mock import AsyncMock, Mock, patch, MagicMock
from datetime import datetime, timezone, timedelta

from fastapi.testclient import TestClient
import httpx

from kiro.config import PROXY_API_KEY


@pytest.mark.parametrize("api", ["anthropic", "openai"])
@pytest.mark.parametrize("stream", [False, True])
def test_sso_fallback_through_both_api_surfaces(
    test_client: TestClient, api: str, stream: bool, monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Preserve replies across conversion, SSO compatibility retry, and SSE parsing."""
    from kiro.auth import KiroAuthManager
    from kiro.http_client import KiroHttpClient

    manager = KiroAuthManager(
        client_id="test-client", client_secret="test-secret",
        profile_arn="cached-profile", api_region="us-east-1",
    )
    manager.get_access_token = AsyncMock(return_value="test-token")
    manager.force_refresh = AsyncMock(return_value="fresh-test-token")
    account = test_client.app.state.account_manager.get_first_account()
    monkeypatch.setattr(account, "auth_manager", manager)
    monkeypatch.setattr(test_client.app.state, "account_system", False)
    upstream = Mock(spec=["request", "build_request", "send"])
    responses = [
        httpx.Response(403, json={"message": "The bearer token included in the request is invalid."}),
        httpx.Response(403, json={"message": "The bearer token included in the request is invalid."}),
        httpx.Response(200, content=b'{"content":"Compatibility OK","modelId":"claude-haiku-4.5"}'),
    ]
    upstream.request = AsyncMock(side_effect=responses)
    upstream.send = AsyncMock(side_effect=responses)
    upstream.build_request = Mock(side_effect=httpx.Request)
    monkeypatch.setattr(KiroHttpClient, "_get_client", AsyncMock(return_value=upstream))
    route = "/v1/messages" if api == "anthropic" else "/v1/chat/completions"
    response = test_client.post(
        route, headers={"Authorization": f"Bearer {PROXY_API_KEY}"},
        json={"model": "claude-haiku-4-5", "max_tokens": 64, "stream": stream,
              "messages": [{"role": "user", "content": "Reply Compatibility OK."}]},
    )
    assert response.status_code == 200, response.text
    if stream:
        assert "Compatibility OK" in response.text
        assert '"type": "error"' not in response.text
    elif api == "anthropic":
        assert response.json()["content"][0]["text"] == "Compatibility OK"
    else:
        assert response.json()["choices"][0]["message"]["content"] == "Compatibility OK"
    calls = upstream.build_request.call_args_list if upstream.send.called else upstream.request.call_args_list
    assert len(calls) == 3
    assert calls[-1].args[1] == "https://q.us-east-1.amazonaws.com/generateAssistantResponse"
    original = json.loads(calls[0].kwargs["content"])
    fallback = json.loads(calls[-1].kwargs["content"])
    assert original.pop("profileArn") == "cached-profile"
    assert fallback == original


class TestFullChatCompletionFlow:
    """Integration tests for complete chat completions flow."""
    
    def test_full_flow_health_to_models_to_chat(self, test_client, valid_proxy_api_key):
        """
        What it does: Checks complete flow from health check to chat completions.
        Goal: Ensure all endpoints work together.
        """
        print("Step 1: Health check...")
        health_response = test_client.get("/health")
        assert health_response.status_code == 200
        assert health_response.json()["status"] == "healthy"
        print(f"Health: {health_response.json()}")
        
        print("Step 2: Getting models list...")
        models_response = test_client.get(
            "/v1/models",
            headers={"Authorization": f"Bearer {valid_proxy_api_key}"}
        )
        assert models_response.status_code == 200
        assert len(models_response.json()["data"]) > 0
        print(f"Models: {[m['id'] for m in models_response.json()['data']]}")
        
        print("Step 3: Validating chat completions request...")
        # This request will pass validation but fail on HTTP due to network blocking
        chat_response = test_client.post(
            "/v1/chat/completions",
            headers={"Authorization": f"Bearer {valid_proxy_api_key}"},
            json={
                "model": "claude-sonnet-4-5",
                "messages": [{"role": "user", "content": "Hello"}]
            }
        )
        # Request should pass validation (not 422)
        assert chat_response.status_code != 422
        print(f"Chat response status: {chat_response.status_code}")
    
    def test_authentication_flow(self, test_client, valid_proxy_api_key, invalid_proxy_api_key):
        """
        What it does: Checks authentication flow.
        Goal: Ensure protected endpoints require authorization.
        """
        print("Step 1: Request without authorization...")
        no_auth_response = test_client.get("/v1/models")
        assert no_auth_response.status_code == 401
        print(f"Without authorization: {no_auth_response.status_code}")
        
        print("Step 2: Request with invalid key...")
        wrong_auth_response = test_client.get(
            "/v1/models",
            headers={"Authorization": f"Bearer {invalid_proxy_api_key}"}
        )
        assert wrong_auth_response.status_code == 401
        print(f"Invalid key: {wrong_auth_response.status_code}")
        
        print("Step 3: Request with valid key...")
        valid_auth_response = test_client.get(
            "/v1/models",
            headers={"Authorization": f"Bearer {valid_proxy_api_key}"}
        )
        assert valid_auth_response.status_code == 200
        print(f"Valid key: {valid_auth_response.status_code}")
    
    def test_openai_compatibility_format(self, test_client, valid_proxy_api_key):
        """
        What it does: Checks response format compatibility with OpenAI API.
        Goal: Ensure responses conform to OpenAI specification.
        """
        print("Checking /v1/models format...")
        models_response = test_client.get(
            "/v1/models",
            headers={"Authorization": f"Bearer {valid_proxy_api_key}"}
        )
        
        assert models_response.status_code == 200
        data = models_response.json()
        
        # Check OpenAI response structure
        assert "object" in data
        assert data["object"] == "list"
        assert "data" in data
        assert isinstance(data["data"], list)
        
        # Check structure of each model
        for model in data["data"]:
            assert "id" in model
            assert "object" in model
            assert model["object"] == "model"
            assert "owned_by" in model
            assert "created" in model
        
        print(f"Format conforms to OpenAI API: {len(data['data'])} models")


class TestRequestValidationFlow:
    """Integration tests for request validation."""
    
    def test_chat_completions_request_validation(self, test_client, valid_proxy_api_key):
        """
        What it does: Checks validation of various request formats.
        Goal: Ensure validation works correctly.
        """
        print("Test 1: Empty messages...")
        empty_messages = test_client.post(
            "/v1/chat/completions",
            headers={"Authorization": f"Bearer {valid_proxy_api_key}"},
            json={"model": "claude-sonnet-4-5", "messages": []}
        )
        assert empty_messages.status_code == 422
        print(f"Empty messages: {empty_messages.status_code}")
        
        print("Test 2: Missing model...")
        no_model = test_client.post(
            "/v1/chat/completions",
            headers={"Authorization": f"Bearer {valid_proxy_api_key}"},
            json={"messages": [{"role": "user", "content": "Hello"}]}
        )
        assert no_model.status_code == 422
        print(f"Without model: {no_model.status_code}")
        
        print("Test 3: Missing messages...")
        no_messages = test_client.post(
            "/v1/chat/completions",
            headers={"Authorization": f"Bearer {valid_proxy_api_key}"},
            json={"model": "claude-sonnet-4-5"}
        )
        assert no_messages.status_code == 422
        print(f"Without messages: {no_messages.status_code}")
        
        print("Test 4: Valid request...")
        valid_request = test_client.post(
            "/v1/chat/completions",
            headers={"Authorization": f"Bearer {valid_proxy_api_key}"},
            json={
                "model": "claude-sonnet-4-5",
                "messages": [{"role": "user", "content": "Hello"}]
            }
        )
        # Validation should pass (not 422)
        assert valid_request.status_code != 422
        print(f"Valid request: {valid_request.status_code}")
    
    def test_complex_message_formats(self, test_client, valid_proxy_api_key):
        """
        What it does: Checks handling of complex message formats.
        Goal: Ensure multimodal and tool formats are accepted.
        """
        print("Test 1: System + User messages...")
        system_user = test_client.post(
            "/v1/chat/completions",
            headers={"Authorization": f"Bearer {valid_proxy_api_key}"},
            json={
                "model": "claude-sonnet-4-5",
                "messages": [
                    {"role": "system", "content": "You are helpful"},
                    {"role": "user", "content": "Hello"}
                ]
            }
        )
        assert system_user.status_code != 422
        print(f"System + User: {system_user.status_code}")
        
        print("Test 2: Multi-turn conversation...")
        multi_turn = test_client.post(
            "/v1/chat/completions",
            headers={"Authorization": f"Bearer {valid_proxy_api_key}"},
            json={
                "model": "claude-sonnet-4-5",
                "messages": [
                    {"role": "user", "content": "Hello"},
                    {"role": "assistant", "content": "Hi there!"},
                    {"role": "user", "content": "How are you?"}
                ]
            }
        )
        assert multi_turn.status_code != 422
        print(f"Multi-turn: {multi_turn.status_code}")
        
        print("Test 3: With tools...")
        with_tools = test_client.post(
            "/v1/chat/completions",
            headers={"Authorization": f"Bearer {valid_proxy_api_key}"},
            json={
                "model": "claude-sonnet-4-5",
                "messages": [{"role": "user", "content": "What's the weather?"}],
                "tools": [{
                    "type": "function",
                    "function": {
                        "name": "get_weather",
                        "description": "Get weather",
                        "parameters": {"type": "object", "properties": {}}
                    }
                }]
            }
        )
        assert with_tools.status_code != 422
        print(f"With tools: {with_tools.status_code}")


class TestErrorHandlingFlow:
    """Integration tests for error handling."""
    
    def test_invalid_json_handling(self, test_client, valid_proxy_api_key):
        """
        What it does: Checks handling of invalid JSON.
        Goal: Ensure invalid JSON returns clear error.
        """
        print("Sending invalid JSON...")
        response = test_client.post(
            "/v1/chat/completions",
            headers={
                "Authorization": f"Bearer {valid_proxy_api_key}",
                "Content-Type": "application/json"
            },
            content=b"not valid json"
        )
        
        assert response.status_code == 422
        print(f"Invalid JSON: {response.status_code}")
    
    def test_wrong_content_type_handling(self, test_client, valid_proxy_api_key):
        """
        What it does: Checks handling of wrong Content-Type.
        Goal: Ensure wrong Content-Type is handled.
        """
        print("Sending with wrong Content-Type...")
        response = test_client.post(
            "/v1/chat/completions",
            headers={
                "Authorization": f"Bearer {valid_proxy_api_key}",
                "Content-Type": "text/plain"
            },
            content=b"Hello"
        )
        
        # Should be validation error
        assert response.status_code == 422
        print(f"Wrong Content-Type: {response.status_code}")


class TestModelsEndpointIntegration:
    """Integration tests for /v1/models endpoint."""
    
    def test_models_returns_all_available_models(self, test_client, valid_proxy_api_key):
        """
        What it does: Checks that all models from config are returned.
        Goal: Ensure completeness of models list.
        """
        print("Getting models list...")
        response = test_client.get(
            "/v1/models",
            headers={"Authorization": f"Bearer {valid_proxy_api_key}"}
        )
        
        assert response.status_code == 200
        
        returned_ids = {m["id"] for m in response.json()["data"]}
        
        print(f"Returned models: {returned_ids}")
        
        # At minimum, hidden models should be available
        assert len(returned_ids) >= 1, "Expected at least one model (hidden models)"
    
    def test_models_caching_behavior(self, test_client, valid_proxy_api_key):
        """
        What it does: Checks models caching behavior.
        Goal: Ensure repeated requests work correctly.
        """
        print("First models request...")
        response1 = test_client.get(
            "/v1/models",
            headers={"Authorization": f"Bearer {valid_proxy_api_key}"}
        )
        assert response1.status_code == 200
        
        print("Second models request...")
        response2 = test_client.get(
            "/v1/models",
            headers={"Authorization": f"Bearer {valid_proxy_api_key}"}
        )
        assert response2.status_code == 200
        
        # Responses should be identical
        assert response1.json()["data"] == response2.json()["data"]
        print("Caching works correctly")


class TestStreamingFlagHandling:
    """Integration tests for stream flag handling."""
    
    def test_stream_true_accepted(self, test_client, valid_proxy_api_key):
        """
        What it does: Checks that stream=true is accepted.
        Goal: Ensure streaming mode is available.
        
        Note: Streaming mode requires HTTP client mock,
        as request is executed inside generator.
        """
        print("Request with stream=true...")
        
        # Create mock response for streaming
        mock_response = AsyncMock()
        mock_response.status_code = 200
        
        async def mock_aiter_bytes():
            yield b'{"content":"Hello"}'
            yield b'{"usage":0.5}'
        
        mock_response.aiter_bytes = mock_aiter_bytes
        mock_response.aclose = AsyncMock()
        
        # Mock request_with_retry to return our mock response
        with patch('kiro.routes_openai.KiroHttpClient') as MockHttpClient:
            mock_client_instance = AsyncMock()
            mock_client_instance.request_with_retry = AsyncMock(return_value=mock_response)
            mock_client_instance.client = AsyncMock()
            mock_client_instance.close = AsyncMock()
            MockHttpClient.return_value = mock_client_instance
            
            response = test_client.post(
                "/v1/chat/completions",
                headers={"Authorization": f"Bearer {valid_proxy_api_key}"},
                json={
                    "model": "claude-sonnet-4-5",
                    "messages": [{"role": "user", "content": "Hello"}],
                    "stream": True
                }
            )
        
        # Validation should pass and streaming should work
        assert response.status_code == 200
        print(f"stream=true: {response.status_code}")
    
    def test_stream_false_accepted(self, test_client, valid_proxy_api_key):
        """
        What it does: Checks that stream=false is accepted.
        Goal: Ensure non-streaming mode is available.
        """
        print("Request with stream=false...")
        response = test_client.post(
            "/v1/chat/completions",
            headers={"Authorization": f"Bearer {valid_proxy_api_key}"},
            json={
                "model": "claude-sonnet-4-5",
                "messages": [{"role": "user", "content": "Hello"}],
                "stream": False
            }
        )
        
        # Validation should pass
        assert response.status_code != 422
        print(f"stream=false: {response.status_code}")


class TestHealthEndpointIntegration:
    """Integration tests for health endpoints."""
    
    def test_root_and_health_consistency(self, test_client):
        """
        What it does: Checks consistency of / and /health.
        Goal: Ensure both endpoints return correct status.
        """
        print("Request to /...")
        root_response = test_client.get("/")
        
        print("Request to /health...")
        health_response = test_client.get("/health")
        
        assert root_response.status_code == 200
        assert health_response.status_code == 200
        
        # Both should show "ok" status
        assert root_response.json()["status"] == "ok"
        assert health_response.json()["status"] == "healthy"
        
        # Versions should match
        assert root_response.json()["version"] == health_response.json()["version"]
        
        print("Health endpoints are consistent")


class TestRequestLogConsistency:
    """
    The same upstream events produce the same request log row on all four paths:
    OpenAI / Anthropic x streaming / non-streaming.

    Only the HTTP client and the Kiro event parser are mocked; routes, converters,
    streaming layers and the request log run for real.
    """

    UPSTREAM_EVENTS = [
        ("content", "Hello"),
        ("metering", 0.0930),
        ("metering", 0.5290),
        ("context_usage", 0.42),
    ]

    @classmethod
    def _parse_stream(cls):
        from kiro.streaming_core import KiroEvent

        async def fake_parse_kiro_stream(*args, **kwargs):
            for kind, value in cls.UPSTREAM_EVENTS:
                if kind == "content":
                    yield KiroEvent(type="content", content=value)
                elif kind == "metering":
                    yield KiroEvent(type="metering", credits=value)
                elif kind == "context_usage":
                    yield KiroEvent(type="context_usage", context_usage_percentage=value)
        return fake_parse_kiro_stream

    @staticmethod
    def _http_client(mock_kiro_http_client_class):
        mock_response = MagicMock()
        mock_response.status_code = 200
        mock_response.aclose = AsyncMock()
        instance = AsyncMock()
        instance.request_with_retry = AsyncMock(return_value=mock_response)
        instance.close = AsyncMock()
        instance.client = MagicMock()
        mock_kiro_http_client_class.return_value = instance

    def _call(self, test_client, api, stream):
        if api == "openai":
            return test_client.post(
                "/v1/chat/completions",
                headers={"Authorization": f"Bearer {PROXY_API_KEY}"},
                json={"model": "claude-sonnet-4-5", "stream": stream, "messages": [{"role": "user", "content": "Hi"}]},
            )
        return test_client.post(
            "/v1/messages",
            headers={"x-api-key": PROXY_API_KEY, "anthropic-version": "2023-06-01"},
            json={"model": "claude-sonnet-4-5", "stream": stream, "max_tokens": 100, "messages": [{"role": "user", "content": "Hi"}]},
        )

    @patch('kiro.routes_anthropic.KiroHttpClient')
    @patch('kiro.routes_openai.KiroHttpClient')
    def test_request_log_consistent_across_four_paths(self, mock_openai_client, mock_anthropic_client, test_client, request_log_dir):
        """
        What it does: Sends one request on each of the four paths with identical upstream events.
        Purpose: Rows differ only in api/stream/id/ts/duration_ms; credits match the [Credit] log.
        """
        self._http_client(mock_openai_client)
        self._http_client(mock_anthropic_client)
        parse_stream = self._parse_stream()

        combinations = [("openai", True), ("openai", False), ("anthropic", True), ("anthropic", False)]
        with patch('kiro.streaming_openai.parse_kiro_stream', parse_stream), \
             patch('kiro.streaming_anthropic.parse_kiro_stream', parse_stream), \
             patch('kiro.streaming_core.parse_kiro_stream', parse_stream):
            for api, stream in combinations:
                response = self._call(test_client, api, stream)
                assert response.status_code == 200, (api, stream, response.text[:200])
                # Ensure the streaming body is fully consumed
                _ = response.text

        rows = request_log_dir.records()
        assert len(rows) == 4
        assert [(r["api"], r["stream"]) for r in rows] == combinations

        varying = {"api", "stream", "id", "ts", "duration_ms"}
        normalized = [{k: v for k, v in row.items() if k not in varying} for row in rows]
        assert all(n == normalized[0] for n in normalized), normalized
        assert normalized[0]["credits"] == pytest.approx(0.6220)
        assert normalized[0]["status"] == 200
        assert normalized[0]["outcome"] == "ok"
        assert all(row["id"] for row in rows)
        assert len({row["id"] for row in rows}) == 4
