"""Exercise Boundscope using the official MCP Python client (2.x)."""
import asyncio
import json
import os
from pathlib import Path
import tempfile

from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

REPO = Path(__file__).resolve().parents[2]


async def main():
    binary = (REPO / "target/debug/boundscope-mcp").resolve()
    assert binary.is_file(), "Run cargo build --locked first"
    with tempfile.TemporaryDirectory(prefix="boundscope official mcp ") as scratch:
        root = Path(scratch)
        source = (REPO / "examples/c/buffer.c").read_text()
        (root / "buffer.c").write_text(source)
        config = {
            "schema": "boundscope.targets.v1",
            "compiler": os.environ.get("BOUNDSCOPE_TEST_CLANG", "/usr/bin/clang"),
            "targets": [{
                "id": "buffer", "root": ".", "language": "c", "sources": ["buffer.c"],
                "cases": [{"id": "boundary", "args": ["4"]}],
                "limits": {"test_timeout_ms": 10000}
            }]
        }
        config_path = root / "targets.json"
        config_path.write_text(json.dumps(config))
        store = root / "reports"
        params = StdioServerParameters(command=str(binary), args=["--config", str(config_path), "--store", str(store)])
        async with stdio_client(params) as (read, write):
            async with ClientSession(read, write) as session:
                initialized = await session.initialize()
                assert initialized.server_info.name == "boundscope"
                discovered = await session.list_tools()
                assert {tool.name for tool in discovered.tools} == {"list_targets", "run_checks", "retest", "list_reports", "get_report", "list_traces", "get_trace"}
                targets = await session.call_tool("list_targets", {})
                assert not targets.is_error
                assert targets.structured_content["targets"][0]["id"] == "buffer"
                result = await session.call_tool("run_checks", {"target": "buffer"}, read_timeout_seconds=60)
                assert not result.is_error, result
                baseline = result.structured_content
                assert baseline["outcome"] == "findings_observed", baseline
                assert baseline["cases"][0]["findings"][0]["kind"] == "heap-buffer-overflow"
                inspected = await session.call_tool("get_report", {"report_id": baseline["id"]})
                assert not inspected.is_error
                assert "AddressSanitizer" in inspected.structured_content["cases"][0]["process"]["stderr"]
                traced = await session.call_tool("get_trace", {"trace_id": baseline["trace_id"]})
                assert not traced.is_error, traced
                assert traced.structured_content["state"] == "report_saved"
                assert traced.structured_content["report_id"] == baseline["id"]
                before = (store / baseline["id"] / "report.json").read_bytes()
                (root / "buffer.c").write_text(source.replace("index <= count", "index < count"))
                retest = await session.call_tool("retest", {"report_id": baseline["id"]}, read_timeout_seconds=60)
                assert not retest.is_error, retest
                assert retest.structured_content["outcome"] == "no_errors_observed"
                assert retest.structured_content["comparison"]["baseline_report_id"] == baseline["id"]
                assert (store / baseline["id"] / "report.json").read_bytes() == before
                denied = await session.call_tool("run_checks", {"target": "buffer", "command": "unregistered"})
                assert denied.is_error
                missing = await session.call_tool("run_checks", {"target": "unknown"})
                assert missing.is_error
                traces = await session.call_tool("list_traces", {"limit": 20})
                failed = [t for t in traces.structured_content["traces"] if t["state"] == "failed"]
                assert len(failed) == 1
                failure = await session.call_tool("get_trace", {"trace_id": failed[0]["trace_id"]})
                assert failure.structured_content["events"][-1]["detail"]["phase"] == "configuration"
                bad_trace = await session.call_tool("get_trace", {"trace_id": "../targets.json"})
                assert bad_trace.is_error
                traversal = await session.call_tool("get_report", {"report_id": "../targets.json"})
                assert traversal.is_error
                listed = await session.call_tool("list_reports", {"limit": 20})
                assert len(listed.structured_content["reports"]) == 2
                config["targets"][0]["cases"][0]["args"] = ["3"]
                config_path.write_text(json.dumps(config))
                changed = await session.call_tool("retest", {"report_id": baseline["id"]}, read_timeout_seconds=60)
                assert changed.is_error
                print("PASS official MCP client: seven tools, real ASan finding, evidence and trace inspection, fixed-source retest, immutable baseline, pre-report failure trace, argument/path guards, changed-definition rejection")


if __name__ == "__main__":
    asyncio.run(main())
