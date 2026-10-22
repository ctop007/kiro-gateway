# -*- coding: utf-8 -*-

"""
Unit tests for the structured per-request log (kiro/request_log.py).

The request log is the source of truth for the desktop app's credit totals,
so these tests focus on the guarantees it relies on: one line per request,
valid JSON per line under concurrency, monthly files, no sensitive content,
and never breaking the request when the disk misbehaves.
"""

import json
import os
import stat
import sys
import threading
from datetime import datetime, timedelta, timezone
from pathlib import Path
from unittest.mock import patch

import pytest

from kiro import request_log
from kiro.request_log import (
    ERROR_SUMMARY_MAX_CHARS,
    RequestRecord,
    finish_request_record,
    request_log_file_for,
    start_request_record,
)


@pytest.fixture(autouse=True)
def reset_warning_state():
    """Reset the 'already warned' flag so each test sees the first-failure warning."""
    request_log._write_failure_warned = False
    yield
    request_log._write_failure_warned = False


def _read_lines(directory: Path) -> list:
    """Read and parse every JSONL line written in the directory."""
    lines = []
    for path in sorted(directory.glob("requests-*.jsonl")):
        for raw in path.read_text(encoding="utf-8").splitlines():
            lines.append(json.loads(raw))
    return lines


class TestRequestLogDisabled:
    """Behavior when REQUEST_LOG_DIR is empty."""

    def test_disabled_does_not_create_files(self, tmp_path):
        """
        What it does: Finishes a record while the log is disabled.
        Purpose: Users who did not opt in must see no files and no side effects.
        """
        target = tmp_path / "logs"
        with patch.object(request_log, "REQUEST_LOG_DIR", ""):
            record = start_request_record(api="anthropic", model="claude-haiku-4.5", stream=False)
            record.status = 200
            finish_request_record(record)

        assert not target.exists()
        assert list(tmp_path.iterdir()) == []

    def test_start_returns_record_even_when_disabled(self):
        """
        What it does: Verifies start_request_record always returns a usable record.
        Purpose: Call sites must not need "if enabled" branches of their own.
        """
        with patch.object(request_log, "REQUEST_LOG_DIR", ""):
            record = start_request_record(api="openai", model="m", stream=True)

        assert isinstance(record, RequestRecord)
        assert record.api == "openai"
        assert record.stream is True


class TestRequestLogWriting:
    """Basic write behavior."""

    def test_creates_missing_directory(self, tmp_path):
        """
        What it does: Points the log at a directory that does not exist yet.
        Purpose: The desktop app passes a fresh app-data path on first run.
        """
        target = tmp_path / "nested" / "logs"
        with patch.object(request_log, "REQUEST_LOG_DIR", str(target)):
            record = start_request_record(api="anthropic", model="m", stream=False)
            record.status = 200
            finish_request_record(record)

        assert target.is_dir()
        assert len(_read_lines(target)) == 1

    def test_writes_all_documented_fields(self, tmp_path):
        """
        What it does: Checks the exact field set of a written record.
        Purpose: The desktop reader depends on these names; extra fields could leak data.
        """
        with patch.object(request_log, "REQUEST_LOG_DIR", str(tmp_path)):
            record = start_request_record(api="openai", model="claude-opus-4.5", stream=True)
            record.response_id = "chatcmpl-abc"
            record.status = 200
            record.credits = 0.42
            finish_request_record(record)

        [line] = _read_lines(tmp_path)
        assert set(line.keys()) == {
            "ts", "id", "api", "model", "stream", "status",
            "outcome", "duration_ms", "credits", "error",
        }
        assert line["id"] == "chatcmpl-abc"
        assert line["api"] == "openai"
        assert line["model"] == "claude-opus-4.5"
        assert line["stream"] is True
        assert line["status"] == 200
        assert line["outcome"] == "ok"
        assert line["credits"] == pytest.approx(0.42)
        assert line["error"] is None
        assert isinstance(line["duration_ms"], int)
        assert line["duration_ms"] >= 0

    def test_ts_is_timezone_aware_iso8601(self, tmp_path):
        """
        What it does: Parses the ts field back into a datetime.
        Purpose: The desktop groups by local day/month, which needs an explicit offset.
        """
        with patch.object(request_log, "REQUEST_LOG_DIR", str(tmp_path)):
            record = start_request_record(api="openai", model="m", stream=False)
            record.status = 200
            finish_request_record(record)

        [line] = _read_lines(tmp_path)
        parsed = datetime.fromisoformat(line["ts"])
        assert parsed.tzinfo is not None

    def test_default_credits_is_zero_and_id_is_null(self, tmp_path):
        """
        What it does: Finishes a record with no credits and no response id.
        Purpose: Requests that fail before reaching Kiro still produce a valid row.
        """
        with patch.object(request_log, "REQUEST_LOG_DIR", str(tmp_path)):
            record = start_request_record(api="anthropic", model="m", stream=False)
            record.status = 422
            record.error = "invalid request"
            finish_request_record(record)

        [line] = _read_lines(tmp_path)
        assert line["credits"] == 0
        assert line["id"] is None

    def test_model_may_be_null(self, tmp_path):
        """
        What it does: Writes a record whose model could not be determined.
        Purpose: 422 validation failures may not carry a parseable model.
        """
        with patch.object(request_log, "REQUEST_LOG_DIR", str(tmp_path)):
            record = start_request_record(api="anthropic", model=None, stream=False)
            record.status = 422
            finish_request_record(record)

        [line] = _read_lines(tmp_path)
        assert line["model"] is None

    def test_unicode_error_is_written_as_utf8(self, tmp_path):
        """
        What it does: Writes an error summary containing non-ASCII text.
        Purpose: Upstream errors may be localized; the file must stay valid UTF-8.
        """
        with patch.object(request_log, "REQUEST_LOG_DIR", str(tmp_path)):
            record = start_request_record(api="openai", model="m", stream=False)
            record.status = 500
            record.error = "上游错误 ошибка"
            finish_request_record(record)

        [line] = _read_lines(tmp_path)
        assert line["error"] == "上游错误 ошибка"

    def test_each_record_is_exactly_one_line(self, tmp_path):
        """
        What it does: Writes an error containing newlines.
        Purpose: A multi-line error must not split one record into several lines.
        """
        with patch.object(request_log, "REQUEST_LOG_DIR", str(tmp_path)):
            record = start_request_record(api="openai", model="m", stream=False)
            record.status = 500
            record.error = "line one\nline two\r\nline three"
            finish_request_record(record)

        files = list(tmp_path.glob("requests-*.jsonl"))
        content = files[0].read_text(encoding="utf-8")
        assert content.count("\n") == 1
        assert content.endswith("\n")


class TestRequestLogOutcome:
    """Outcome derivation."""

    @pytest.mark.parametrize(
        "status, explicit, expected",
        [
            (200, None, "ok"),
            (402, None, "error"),
            (500, None, "error"),
            (422, None, "error"),
            (200, "refused", "refused"),
            (200, "client_disconnected", "client_disconnected"),
            (500, "error", "error"),
        ],
    )
    def test_outcome(self, tmp_path, status, explicit, expected):
        """
        What it does: Checks outcome for each status/explicit-outcome combination.
        Purpose: Refusals and disconnects return 200 but must not count as plain success.
        """
        with patch.object(request_log, "REQUEST_LOG_DIR", str(tmp_path)):
            record = start_request_record(api="openai", model="m", stream=True)
            record.status = status
            if explicit:
                record.outcome = explicit
            finish_request_record(record)

        [line] = _read_lines(tmp_path)
        assert line["outcome"] == expected

    def test_streaming_error_after_200_is_error(self, tmp_path):
        """
        What it does: A stream that started with 200 then failed mid-way.
        Purpose: The HTTP status stays 200 but the row must show the failure.
        """
        with patch.object(request_log, "REQUEST_LOG_DIR", str(tmp_path)):
            record = start_request_record(api="anthropic", model="m", stream=True)
            record.status = 200
            record.mark_error("upstream connection reset")
            finish_request_record(record)

        [line] = _read_lines(tmp_path)
        assert line["status"] == 200
        assert line["outcome"] == "error"
        assert line["error"] == "upstream connection reset"

    def test_missing_status_is_written_as_500_error(self, tmp_path):
        """
        What it does: Finishes a record whose status was never set.
        Purpose: A forgotten exit path must still show up as a failure, not as success.
        """
        with patch.object(request_log, "REQUEST_LOG_DIR", str(tmp_path)):
            record = start_request_record(api="openai", model="m", stream=False)
            finish_request_record(record)

        [line] = _read_lines(tmp_path)
        assert line["status"] == 500
        assert line["outcome"] == "error"


class TestRequestLogErrorSummary:
    """Error summary sanitizing."""

    def test_error_is_truncated(self, tmp_path):
        """
        What it does: Writes a very long error message.
        Purpose: Upstream may echo request content; keep at most ERROR_SUMMARY_MAX_CHARS.
        """
        with patch.object(request_log, "REQUEST_LOG_DIR", str(tmp_path)):
            record = start_request_record(api="openai", model="m", stream=False)
            record.status = 400
            record.error = "x" * 5000
            finish_request_record(record)

        [line] = _read_lines(tmp_path)
        assert ERROR_SUMMARY_MAX_CHARS == 200
        assert len(line["error"]) <= ERROR_SUMMARY_MAX_CHARS

    def test_empty_error_becomes_null(self, tmp_path):
        """
        What it does: Sets an empty error string on a failed request.
        Purpose: Empty strings are noise for the viewer; null means "no detail".
        """
        with patch.object(request_log, "REQUEST_LOG_DIR", str(tmp_path)):
            record = start_request_record(api="openai", model="m", stream=False)
            record.status = 500
            record.error = "   "
            finish_request_record(record)

        [line] = _read_lines(tmp_path)
        assert line["error"] is None

    def test_record_has_no_content_fields(self, tmp_path):
        """
        What it does: Verifies RequestRecord cannot carry message content.
        Purpose: The spec forbids content, headers and tokens in the log.
        """
        with patch.object(request_log, "REQUEST_LOG_DIR", str(tmp_path)):
            record = start_request_record(api="openai", model="m", stream=False)
            record.status = 200
            finish_request_record(record)

        [line] = _read_lines(tmp_path)
        forbidden = {"messages", "content", "system", "tools", "headers", "authorization", "token", "prompt"}
        assert forbidden.isdisjoint(line.keys())


class TestRequestLogFinishOnce:
    """A record must be written at most once."""

    def test_finish_twice_writes_one_line(self, tmp_path):
        """
        What it does: Calls finish twice on the same record.
        Purpose: Route exit paths and the safety-net finally can both fire.
        """
        with patch.object(request_log, "REQUEST_LOG_DIR", str(tmp_path)):
            record = start_request_record(api="openai", model="m", stream=False)
            record.status = 200
            finish_request_record(record)
            record.status = 500
            finish_request_record(record)

        lines = _read_lines(tmp_path)
        assert len(lines) == 1
        assert lines[0]["status"] == 200

    def test_finished_flag(self, tmp_path):
        """
        What it does: Checks the finished flag before and after finish.
        Purpose: The safety net decides whether to write by reading this flag.
        """
        with patch.object(request_log, "REQUEST_LOG_DIR", str(tmp_path)):
            record = start_request_record(api="openai", model="m", stream=False)
            assert record.finished is False
            record.status = 200
            finish_request_record(record)
            assert record.finished is True

    def test_finish_none_is_noop(self, tmp_path):
        """
        What it does: Passes None to finish.
        Purpose: Streaming helpers receive an optional record and may forward None.
        """
        with patch.object(request_log, "REQUEST_LOG_DIR", str(tmp_path)):
            finish_request_record(None)

        assert list(tmp_path.iterdir()) == []


class TestRequestLogMonthlyFiles:
    """Monthly file naming and rotation."""

    def test_file_name_uses_local_month_of_start_time(self, tmp_path):
        """
        What it does: Builds the file path for a known timestamp.
        Purpose: The desktop app relies on the requests-YYYY-MM.jsonl naming.
        """
        ts = datetime(2026, 10, 31, 23, 59, 30, tzinfo=timezone(timedelta(hours=8)))
        assert request_log_file_for(str(tmp_path), ts) == tmp_path / "requests-2026-10.jsonl"

    def test_month_boundary_splits_files(self, tmp_path):
        """
        What it does: Writes one record at 23:59 on the last day and one at 00:00 next month.
        Purpose: A long-running gateway must roll over without restarting.
        """
        tz = timezone(timedelta(hours=8))
        before = datetime(2026, 10, 31, 23, 59, 0, tzinfo=tz)
        after = datetime(2026, 11, 1, 0, 0, 0, tzinfo=tz)

        with patch.object(request_log, "REQUEST_LOG_DIR", str(tmp_path)):
            for started in (before, after):
                with patch.object(request_log, "_now", return_value=started):
                    record = start_request_record(api="openai", model="m", stream=False)
                record.status = 200
                finish_request_record(record)

        assert (tmp_path / "requests-2026-10.jsonl").exists()
        assert (tmp_path / "requests-2026-11.jsonl").exists()

    def test_long_request_stays_in_start_month(self, tmp_path):
        """
        What it does: Starts a request before midnight of month end and finishes after.
        Purpose: A request belongs to the month it started in, matching the spec.
        """
        tz = timezone(timedelta(hours=8))
        with patch.object(request_log, "REQUEST_LOG_DIR", str(tmp_path)):
            with patch.object(request_log, "_now", return_value=datetime(2026, 10, 31, 23, 59, 59, tzinfo=tz)):
                record = start_request_record(api="openai", model="m", stream=True)
            record.status = 200
            with patch.object(request_log, "_now", return_value=datetime(2026, 11, 1, 0, 5, 0, tzinfo=tz)):
                finish_request_record(record)

        assert (tmp_path / "requests-2026-10.jsonl").exists()
        assert not (tmp_path / "requests-2026-11.jsonl").exists()

    def test_recreates_file_deleted_while_running(self, tmp_path):
        """
        What it does: Deletes the month file between two writes.
        Purpose: The desktop app prunes old files; the gateway must never hold a stale handle.
        """
        with patch.object(request_log, "REQUEST_LOG_DIR", str(tmp_path)):
            first = start_request_record(api="openai", model="m", stream=False)
            first.status = 200
            finish_request_record(first)

            for path in tmp_path.glob("requests-*.jsonl"):
                path.unlink()

            second = start_request_record(api="openai", model="m", stream=False)
            second.status = 200
            finish_request_record(second)

        assert len(_read_lines(tmp_path)) == 1

    def test_appends_to_existing_file(self, tmp_path):
        """
        What it does: Writes into a month file that already has content.
        Purpose: Restarting the gateway must keep history, not truncate it.
        """
        with patch.object(request_log, "REQUEST_LOG_DIR", str(tmp_path)):
            for _ in range(3):
                record = start_request_record(api="openai", model="m", stream=False)
                record.status = 200
                finish_request_record(record)

        assert len(_read_lines(tmp_path)) == 3


class TestRequestLogWriteFailures:
    """Write failures must never break the request."""

    @pytest.mark.skipif(sys.platform == "win32", reason="POSIX permissions")
    @pytest.mark.skipif(hasattr(os, "geteuid") and os.geteuid() == 0, reason="root ignores permissions")
    def test_unwritable_directory_does_not_raise(self, tmp_path):
        """
        What it does: Points the log at a read-only directory.
        Purpose: A broken log destination must not turn a successful request into a 500.
        """
        target = tmp_path / "ro"
        target.mkdir()
        target.chmod(stat.S_IRUSR | stat.S_IXUSR)
        try:
            with patch.object(request_log, "REQUEST_LOG_DIR", str(target)):
                record = start_request_record(api="openai", model="m", stream=False)
                record.status = 200
                finish_request_record(record)
        finally:
            target.chmod(stat.S_IRWXU)

        assert list(target.iterdir()) == []

    def test_path_is_a_file_does_not_raise(self, tmp_path):
        """
        What it does: Sets REQUEST_LOG_DIR to an existing regular file.
        Purpose: Misconfiguration must degrade to "no log", not to failed requests.
        """
        not_a_dir = tmp_path / "file.txt"
        not_a_dir.write_text("x")
        with patch.object(request_log, "REQUEST_LOG_DIR", str(not_a_dir)):
            record = start_request_record(api="openai", model="m", stream=False)
            record.status = 200
            finish_request_record(record)

        assert not_a_dir.read_text() == "x"

    def test_warns_only_once(self, tmp_path):
        """
        What it does: Fails to write several times in a row.
        Purpose: A broken destination must not flood the log with one warning per request.
        """
        not_a_dir = tmp_path / "file.txt"
        not_a_dir.write_text("x")
        with patch.object(request_log, "REQUEST_LOG_DIR", str(not_a_dir)):
            with patch.object(request_log.logger, "warning") as warning:
                for _ in range(5):
                    record = start_request_record(api="openai", model="m", stream=False)
                    record.status = 200
                    finish_request_record(record)

        assert warning.call_count == 1

    def test_unexpected_os_error_marks_finished(self, tmp_path):
        """
        What it does: Simulates a disk-full error during write.
        Purpose: The record must still count as finished so the safety net does not retry forever.
        """
        with patch.object(request_log, "REQUEST_LOG_DIR", str(tmp_path)):
            record = start_request_record(api="openai", model="m", stream=False)
            record.status = 200
            with patch("builtins.open", side_effect=OSError(28, "No space left on device")):
                finish_request_record(record)

        assert record.finished is True


class TestRequestLogConcurrency:
    """Concurrent writers."""

    def test_concurrent_writes_produce_valid_lines(self, tmp_path):
        """
        What it does: Finishes 20 records from 20 threads at once, each with a long error.
        Purpose: Lines must never interleave; every line must parse on its own.
        """
        barrier = threading.Barrier(20)

        def worker(index: int) -> None:
            record = start_request_record(api="openai", model=f"model-{index}", stream=True)
            record.status = 500
            record.error = f"{index}-" + "e" * 150
            barrier.wait()
            finish_request_record(record)

        with patch.object(request_log, "REQUEST_LOG_DIR", str(tmp_path)):
            threads = [threading.Thread(target=worker, args=(i,)) for i in range(20)]
            for thread in threads:
                thread.start()
            for thread in threads:
                thread.join()

        lines = _read_lines(tmp_path)
        assert len(lines) == 20
        assert {line["model"] for line in lines} == {f"model-{i}" for i in range(20)}


class TestTrackRequest:
    """track_request() owns every exit of a route."""

    @pytest.mark.asyncio
    async def test_json_success(self, tmp_path):
        """
        What it does: Handler returns a 200 JSONResponse.
        Purpose: Non-streaming success is recorded with status 200 and outcome ok.
        """
        from fastapi.responses import JSONResponse
        from kiro.request_log import track_request

        with patch.object(request_log, "REQUEST_LOG_DIR", str(tmp_path)):
            record = start_request_record(api="openai", model="m", stream=False)

            async def handler():
                return JSONResponse(content={"ok": True})

            response = await track_request(record, handler)

        assert response.status_code == 200
        [line] = _read_lines(tmp_path)
        assert line["status"] == 200
        assert line["outcome"] == "ok"
        assert line["error"] is None

    @pytest.mark.asyncio
    @pytest.mark.parametrize(
        "body, expected",
        [
            ({"error": {"type": "api_error", "message": "Monthly request limit exceeded"}}, "Monthly request limit exceeded"),
            ({"type": "error", "error": {"type": "api_error", "message": "Bad model"}}, "Bad model"),
            ({"detail": "No initialized accounts available"}, "No initialized accounts available"),
        ],
    )
    async def test_json_error_body_is_summarized(self, tmp_path, body, expected):
        """
        What it does: Handler returns an error JSONResponse in each known shape.
        Purpose: The error message is taken from the body the client actually got.
        """
        from fastapi.responses import JSONResponse
        from kiro.request_log import track_request

        with patch.object(request_log, "REQUEST_LOG_DIR", str(tmp_path)):
            record = start_request_record(api="anthropic", model="m", stream=False)

            async def handler():
                return JSONResponse(status_code=402, content=body)

            await track_request(record, handler)

        [line] = _read_lines(tmp_path)
        assert line["status"] == 402
        assert line["outcome"] == "error"
        assert line["error"] == expected

    @pytest.mark.asyncio
    async def test_http_exception_is_recorded_and_reraised(self, tmp_path):
        """
        What it does: Handler raises HTTPException(503).
        Purpose: The exception still reaches FastAPI and the row carries its status/detail.
        """
        from fastapi import HTTPException
        from kiro.request_log import track_request

        with patch.object(request_log, "REQUEST_LOG_DIR", str(tmp_path)):
            record = start_request_record(api="openai", model="m", stream=False)

            async def handler():
                raise HTTPException(status_code=503, detail="Account unavailable")

            with pytest.raises(HTTPException):
                await track_request(record, handler)

        [line] = _read_lines(tmp_path)
        assert line["status"] == 503
        assert line["error"] == "Account unavailable"

    @pytest.mark.asyncio
    async def test_unexpected_exception_is_recorded_as_500(self, tmp_path):
        """
        What it does: Handler raises a non-HTTP exception.
        Purpose: The exception propagates unchanged and the row shows a 500 failure.
        """
        from kiro.request_log import track_request

        with patch.object(request_log, "REQUEST_LOG_DIR", str(tmp_path)):
            record = start_request_record(api="openai", model="m", stream=False)

            async def handler():
                raise RuntimeError("boom")

            with pytest.raises(RuntimeError):
                await track_request(record, handler)

        [line] = _read_lines(tmp_path)
        assert line["status"] == 500
        assert line["outcome"] == "error"

    @pytest.mark.asyncio
    async def test_stream_owned_response_is_not_finished_early(self, tmp_path):
        """
        What it does: Handler marks stream_owned and returns a StreamingResponse.
        Purpose: The route generator, not track_request, writes the streaming row.
        """
        from fastapi.responses import StreamingResponse
        from kiro.request_log import track_request

        with patch.object(request_log, "REQUEST_LOG_DIR", str(tmp_path)):
            record = start_request_record(api="openai", model="m", stream=True)

            async def body():
                yield "data: x\n\n"

            async def handler():
                record.stream_owned = True
                return StreamingResponse(body(), media_type="text/event-stream")

            response = await track_request(record, handler)

            assert record.finished is False
            assert response.body_iterator is not None
        assert list(tmp_path.glob("requests-*.jsonl")) == []

    @pytest.mark.asyncio
    async def test_foreign_streaming_response_is_finished_after_stream(self, tmp_path):
        """
        What it does: Handler returns a StreamingResponse it does not own (native web_search).
        Purpose: The row is written once the wrapped body is fully consumed.
        """
        from fastapi.responses import StreamingResponse
        from kiro.request_log import track_request

        with patch.object(request_log, "REQUEST_LOG_DIR", str(tmp_path)):
            record = start_request_record(api="anthropic", model="m", stream=True)

            async def body():
                yield "event: a\n\n"
                yield "event: b\n\n"

            async def handler():
                return StreamingResponse(body(), media_type="text/event-stream")

            response = await track_request(record, handler)
            assert list(tmp_path.glob("requests-*.jsonl")) == []

            chunks = [chunk async for chunk in response.body_iterator]

        assert chunks == ["event: a\n\n", "event: b\n\n"]
        [line] = _read_lines(tmp_path)
        assert line["status"] == 200
        assert line["outcome"] == "ok"

    @pytest.mark.asyncio
    async def test_foreign_streaming_response_failure_is_error(self, tmp_path):
        """
        What it does: A wrapped streaming body raises mid-way.
        Purpose: A broken foreign stream is recorded as an error, not as success.
        """
        from fastapi.responses import StreamingResponse
        from kiro.request_log import track_request

        with patch.object(request_log, "REQUEST_LOG_DIR", str(tmp_path)):
            record = start_request_record(api="anthropic", model="m", stream=True)

            async def body():
                yield "event: a\n\n"
                raise RuntimeError("upstream reset")

            async def handler():
                return StreamingResponse(body(), media_type="text/event-stream")

            response = await track_request(record, handler)
            with pytest.raises(RuntimeError):
                async for _ in response.body_iterator:
                    pass

        [line] = _read_lines(tmp_path)
        assert line["outcome"] == "error"
