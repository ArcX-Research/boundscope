"""CLI and raw stdio lifecycle regressions; requires only the Python standard library."""
import json
import os
from pathlib import Path
import queue
import re
import subprocess
import tempfile
import threading
import time
import unittest

REPO = Path(__file__).resolve().parents[2]
CLI = REPO / "target/debug/boundscope"
MCP = REPO / "target/debug/boundscope-mcp"


class Session:
    def __init__(self, config, store):
        self.process = subprocess.Popen([str(MCP), "--config", str(config), "--store", str(store)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.received = queue.Queue()
        self.reader = threading.Thread(target=self._read, daemon=True)
        self.reader.start()

    def _read(self):
        for line in self.process.stdout:
            try:
                self.received.put(json.loads(line))
            except ValueError as error:
                self.received.put(error)

    def send(self, method, params=None, request_id=None):
        message = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            message["params"] = params
        if request_id is not None:
            message["id"] = request_id
        self.raw(json.dumps(message) + "\n")

    def raw(self, text):
        self.process.stdin.write(text)
        self.process.stdin.flush()

    def receive(self, request_id, timeout=30):
        result = self.received.get(timeout=timeout)
        if isinstance(result, Exception):
            raise result
        assert result.get("id") == request_id, result
        return result

    def initialize(self):
        self.send("initialize", {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}, 1)
        assert self.receive(1)["result"]["serverInfo"]["name"] == "boundscope"
        self.send("notifications/initialized")

    def close(self):
        if not self.process.stdin.closed:
            self.process.stdin.close()
        try:
            self.process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait(timeout=5)
            raise
        self.reader.join(timeout=2)
        errors = self.process.stderr.read()
        self.process.stdout.close()
        self.process.stderr.close()
        assert self.process.returncode == 0, errors


class Interfaces(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory(prefix="boundscope interfaces ")
        self.addCleanup(self.scratch.cleanup)
        self.root = Path(self.scratch.name)
        (self.root / "cases.c").write_text((REPO / "tests/fixtures/c/cases.c").read_text())
        self.config = self.root / "targets.json"
        self.store = self.root / "reports"
        self.document = {"schema": "boundscope.targets.v1", "compiler": os.environ.get("BOUNDSCOPE_TEST_CLANG", "/usr/bin/clang"), "targets": [{"id": "fixture", "root": ".", "language": "c", "sources": ["cases.c"], "cases": [{"id": "safe", "args": ["safe"], "stdin": "hello\n"}], "limits": {"test_timeout_ms": 10000}}]}
        self.save_config()

    def save_config(self):
        self.config.write_text(json.dumps(self.document))

    def session(self):
        session = Session(self.config, self.store)
        self.addCleanup(session.close)
        return session

    def cli(self, *args, env=None):
        # A bare relative config filename exercises default working-directory resolution.
        return subprocess.run([str(CLI), "--config", "targets.json", "--store", str(self.store), *args], cwd=self.root, env=env, text=True, capture_output=True, timeout=60)

    def test_cli_exit_codes_and_real_diagnostics(self):
        clean = self.cli("check", "fixture")
        self.assertEqual(clean.returncode, 0, clean.stderr)
        self.assertEqual(json.loads(clean.stdout)["outcome"], "no_errors_observed")
        self.document["targets"][0]["cases"] = [{"id": "oob", "args": ["oob"]}]
        self.save_config()
        fault = self.cli("check", "fixture")
        self.assertEqual(fault.returncode, 1, fault.stderr)
        report_id = json.loads(fault.stdout)["id"]
        report = self.cli("report", report_id)
        self.assertEqual(report.returncode, 0, report.stderr)
        self.assertIn("heap-buffer-overflow", report.stdout)
        (self.root / "cases.c").write_text("bad C source")
        incomplete = self.cli("check", "fixture")
        self.assertEqual(incomplete.returncode, 2)
        self.assertEqual(json.loads(incomplete.stdout)["outcome"], "incomplete")
        missing = self.cli("check", "unregistered")
        self.assertEqual(missing.returncode, 2)
        self.assertIn("unknown registered target", missing.stderr)

    def test_malformed_protocol_and_initialization(self):
        session = self.session()
        session.raw("{\n")
        self.assertEqual(session.receive(None)["error"]["code"], -32700)
        session.raw("[]\n")
        self.assertEqual(session.receive(None)["error"]["code"], -32600)
        session.send("tools/list", {}, 2)
        self.assertEqual(session.receive(2)["error"]["code"], -32600)
        session.initialize()
        session.send("tools/list", {}, 3)
        self.assertEqual(len(session.receive(3)["result"]["tools"]), 7)
        session.raw('{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"run_checks","arguments":{"target":"fixture","target":"other"}}}\n')
        self.assertEqual(session.receive(None)["error"]["code"], -32700)
        self.assertFalse(list(self.store.glob("traces/*.jsonl")))
        session.raw(" " * (1024 * 1024 + 1) + "\n")
        self.assertEqual(session.receive(None)["error"]["code"], -32700)
        session.send("tools/call", {"name": "unknown", "arguments": {}}, 4)
        self.assertEqual(session.receive(4)["error"]["code"], -32602)
        session.send("ping", {}, 5)
        self.assertEqual(session.receive(5)["result"], {})

    def start_long_case(self, session):
        self.document["targets"][0]["cases"] = [{"id": "child", "args": ["child"]}]
        self.document["targets"][0]["limits"]["test_timeout_ms"] = 30000
        self.save_config()
        session.initialize()
        session.send("tools/call", {"name": "run_checks", "arguments": {"target": "fixture"}}, 10)
        until = time.monotonic() + 20
        while not list(self.store.glob(".work-*/case-child/descendant.pid")):
            self.assertLess(time.monotonic(), until)
            time.sleep(0.02)

    def assert_cancelled_report(self):
        until = time.monotonic() + 10
        reports = []
        while not reports:
            # A staging report exists briefly before its atomic publication.
            # Only a final report ID is evidence the run has been published.
            reports = [p for p in self.store.glob("*/report.json") if re.fullmatch(r"[0-9a-f]{32}", p.parent.name)]
            self.assertLess(time.monotonic(), until)
            time.sleep(0.02)
        report = json.loads(reports[0].read_text())
        self.assertEqual(report["execution"], "cancelled")
        self.assertEqual(report["outcome"], "incomplete")
        trace = self.cli("trace", report["trace_id"])
        self.assertEqual(trace.returncode, 0, trace.stderr)
        self.assertEqual(json.loads(trace.stdout)["state"], "report_saved")
        self.assertFalse(list(self.store.glob(".work-*")))
        self.assertEqual(len(report["cases"]), 1)
        self.assertEqual(report["cases"][0]["status"], "cancelled")
        child_pid = int(report["cases"][0]["process"]["stdout"].strip())
        # A killed orphan can briefly remain a zombie until the OS reaps it.
        state = subprocess.run(["ps", "-o", "stat=", "-p", str(child_pid)], capture_output=True, text=True).stdout.strip()
        self.assertTrue(not state or state.startswith("Z"), f"descendant still running: {state}")

    def test_cancellation_and_read_tools_during_execution(self):
        session = self.session()
        self.start_long_case(session)
        session.send("tools/call", {"name": "list_targets", "arguments": {}}, 11)
        self.assertFalse(session.receive(11)["result"]["isError"])
        session.send("tools/call", {"name": "list_traces", "arguments": {}}, 14)
        trace = session.receive(14)["result"]["structuredContent"]["traces"][0]
        self.assertEqual(trace["state"], "unfinished")
        session.send("tools/call", {"name": "get_trace", "arguments": {"trace_id": trace["trace_id"]}}, 15)
        live_trace = session.receive(15)["result"]["structuredContent"]
        self.assertEqual(live_trace["events"][-1]["detail"]["phase"], "case")
        session.send("tools/call", {"name": "run_checks", "arguments": {"target": "fixture"}}, 12)
        self.assertTrue(session.receive(12)["result"]["isError"])
        session.send("notifications/cancelled", {"requestId": 10, "reason": "test cancellation"})
        self.assert_cancelled_report()
        session.send("ping", {}, 13)
        self.assertEqual(session.receive(13)["result"], {})

    def test_disconnect_cancels_process_group(self):
        session = self.session()
        self.start_long_case(session)
        session.process.stdin.close()
        session.process.wait(timeout=10)
        self.assert_cancelled_report()

    def test_environment_isolation_and_trace_privacy(self):
        (self.root / "cases.c").write_text('''#include <stdio.h>
#include <stdlib.h>
#include <string.h>
int main(int argc, char **argv) {
    const char *asan = getenv("ASAN_OPTIONS");
    if (getenv("BOUNDSCOPE_TEST_CREDENTIAL") || !asan || !strstr(asan, "halt_on_error=1")) return 9;
    if (argc > 1) puts(argv[1]);
    char line[100];
    if (fgets(line, sizeof(line), stdin)) fputs(line, stderr);
    puts("source-canary-456");
    return 0;
}
''')
        self.document["targets"][0]["cases"] = [{"id": "private-case", "args": ["argument-canary-123"], "stdin": "input-canary-789\n"}]
        self.save_config()
        environment = dict(os.environ, BOUNDSCOPE_TEST_CREDENTIAL="environment-canary-000", ASAN_OPTIONS="halt_on_error=0")
        completed = self.cli("check", "fixture", env=environment)
        self.assertEqual(completed.returncode, 0, completed.stderr)
        report = json.loads(completed.stdout)
        saved = (self.store / report["id"] / "report.json").read_text()
        self.assertIn("argument-canary-123", saved)
        self.assertIn("source-canary-456", saved)
        trace_result = self.cli("trace", report["trace_id"])
        self.assertEqual(trace_result.returncode, 0, trace_result.stderr)
        trace_text = trace_result.stdout
        self.assertEqual(json.loads(trace_text)["state"], "report_saved")
        for secret in ["argument-canary-123", "source-canary-456", "input-canary-789", "environment-canary-000", "private-case", str(self.root)]:
            self.assertNotIn(secret, trace_text)
        self.assertNotIn("environment-canary-000", saved)
        self.assertEqual(len(json.loads(self.cli("list-traces").stdout)["traces"]), 1)
        self.assertEqual(self.cli("trace", "../targets.json").returncode, 2)


if __name__ == "__main__":
    unittest.main(verbosity=2)
