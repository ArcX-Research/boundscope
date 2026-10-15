use boundscope::{config::Catalog, engine::Engine, report::Store};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

struct Fixture {
    dir: tempfile::TempDir,
    config: PathBuf,
    engine: Engine,
}
impl Fixture {
    fn new(source: &str, cases: Vec<Value>) -> Self {
        let dir = tempfile::Builder::new()
            .prefix("boundscope test ")
            .tempdir()
            .unwrap();
        fs::write(dir.path().join("main.c"), source).unwrap();
        let config = dir.path().join("targets.json");
        let compiler =
            std::env::var("BOUNDSCOPE_TEST_CLANG").unwrap_or_else(|_| "/usr/bin/clang".into());
        let value = json!({"schema":"boundscope.targets.v1","compiler":compiler,"targets":[{
            "id":"test","root":".","language":"c","sources":["main.c"],"cases":cases,
            "limits":{"build_timeout_ms":30000,"test_timeout_ms":10000,"output_bytes":65536}
        }]});
        fs::write(&config, serde_json::to_vec(&value).unwrap()).unwrap();
        let engine = Engine::new(config.clone(), dir.path().join("reports")).unwrap();
        Self {
            dir,
            config,
            engine,
        }
    }
    fn edit_config(&self, edit: impl FnOnce(&mut Value)) {
        let mut value: Value = serde_json::from_slice(&fs::read(&self.config).unwrap()).unwrap();
        edit(&mut value);
        fs::write(&self.config, serde_json::to_vec(&value).unwrap()).unwrap();
    }
    fn check(&self) -> boundscope::report::Report {
        self.engine.check("test", &AtomicBool::new(false)).unwrap()
    }
}
fn case(name: &str) -> Value {
    json!({"id":name,"args":[name],"stdin":"hello\n"})
}

#[test]
fn real_sanitizers_distinguish_controls_and_both_error_families() {
    let fixture = Fixture::new(
        include_str!("fixtures/c/cases.c"),
        vec![case("safe"), case("oob"), case("uaf"), case("overflow")],
    );
    let report = fixture.check();
    assert!(report.build.succeeded(), "{}", report.build.stderr);
    assert_eq!(report.execution, "complete", "{:#?}", report.cases);
    assert_eq!(report.outcome, "findings_observed");
    assert_eq!(report.cases[0].status, "no_error_observed");
    assert_eq!(report.cases[0].process.stdout, "received: hello\n");
    assert_eq!(report.cases[1].findings[0].kind, "heap-buffer-overflow");
    assert_eq!(report.cases[2].findings[0].kind, "heap-use-after-free");
    assert_eq!(
        report.cases[3].findings[0].sanitizer,
        "UndefinedBehaviorSanitizer"
    );
    assert_eq!(fixture.engine.store.load(&report.id).unwrap().id, report.id);
    assert_eq!(fixture.engine.store.list(20).unwrap().len(), 1);
    let trace = boundscope::trace::load(&fixture.engine.store, &report.id).unwrap();
    assert_eq!(report.trace_id.as_deref(), Some(report.id.as_str()));
    assert_eq!(trace.state, "report_saved");
    assert_eq!(trace.report_id.as_deref(), Some(report.id.as_str()));
    assert!(trace.events.iter().any(|record| matches!(
        &record.detail,
        boundscope::trace::Event::ProcessFinished {
            phase: boundscope::trace::Phase::Case,
            case_index: Some(3),
            ..
        }
    )));
    let mode = fs::metadata(
        fixture
            .engine
            .store
            .root
            .join(&report.id)
            .join("report.json"),
    )
    .unwrap()
    .permissions()
    .mode();
    assert_eq!(mode & 0o777, 0o600);
    assert_eq!(
        fs::metadata(fixture.engine.store.root.join(&report.id))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
}

#[test]
fn fixed_source_retest_preserves_baseline_and_rejects_changed_inputs() {
    let source = include_str!("../examples/c/buffer.c");
    let fixture = Fixture::new(source, vec![json!({"id":"boundary","args":["4"]})]);
    let baseline = fixture.check();
    assert_eq!(baseline.outcome, "findings_observed", "{baseline:#?}");
    let saved = fixture
        .engine
        .store
        .root
        .join(&baseline.id)
        .join("report.json");
    let original = fs::read(&saved).unwrap();
    fs::write(
        fixture.dir.path().join("main.c"),
        source.replace("index <= count", "index < count"),
    )
    .unwrap();
    let fixed = fixture
        .engine
        .retest(&baseline.id, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(fixed.outcome, "no_errors_observed", "{fixed:#?}");
    assert_ne!(baseline.id, fixed.id);
    assert_eq!(
        fixed.comparison.as_ref().unwrap().baseline_report_id,
        baseline.id
    );
    assert!(
        fixed.comparison.unwrap().cases[0]
            .observation
            .contains("not observed")
    );
    assert_eq!(fs::read(&saved).unwrap(), original);
    fixture.edit_config(|v| v["targets"][0]["cases"][0]["args"] = json!(["3"]));
    assert!(
        fixture
            .engine
            .retest(&baseline.id, &AtomicBool::new(false))
            .unwrap_err()
            .contains("changed")
    );
    assert_eq!(fixture.engine.store.list(20).unwrap().len(), 2);
}

#[test]
fn incomplete_executions_never_become_clean_results() {
    let fixture = Fixture::new(
        include_str!("fixtures/c/cases.c"),
        vec![case("exit"), case("flood")],
    );
    fixture.edit_config(|v| {
        v["targets"][0]["limits"]["output_bytes"] = json!(1024);
    });
    let report = fixture.check();
    assert_eq!(report.outcome, "incomplete", "{report:#?}");
    assert_eq!(report.cases[0].status, "execution_failed");
    assert_eq!(report.cases[0].process.exit_code, Some(7));
    assert_eq!(report.cases[1].status, "output_limit");
    assert!(report.cases[1].process.output_truncated);
    assert!(report.cases[1].process.captured_bytes <= 1024);
    // Give ordinary exits time to start under concurrent sanitizer builds. Apply
    // the deliberately short deadline only to the case expected to time out.
    fixture.edit_config(|v| {
        v["targets"][0]["cases"] = json!([case("timeout")]);
        v["targets"][0]["limits"]["test_timeout_ms"] = json!(100);
    });
    let timeout = fixture.check();
    assert!(timeout.build.succeeded());
    assert_eq!(timeout.outcome, "incomplete");
    assert_eq!(timeout.cases[0].status, "timed_out");
}

#[test]
fn cpp_example_detects_a_dangling_read_and_retests_the_lifetime_fix() {
    let source = include_str!("../examples/cpp/lifetime.cpp");
    let fixture = Fixture::new(
        source,
        vec![
            json!({"id":"live-owner","args":["keep-alive"]}),
            json!({"id":"released-owner","args":["release-early"]}),
        ],
    );
    fixture.edit_config(|v| v["targets"][0]["language"] = json!("cpp"));
    let baseline = fixture.check();
    assert!(baseline.build.succeeded(), "{}", baseline.build.stderr);
    assert_eq!(baseline.execution, "complete");
    assert_eq!(baseline.outcome, "findings_observed");
    assert_eq!(baseline.cases[0].status, "no_error_observed");
    assert_eq!(baseline.cases[0].process.stdout, "42\n");
    assert_eq!(baseline.cases[1].status, "sanitizer_error");
    assert_eq!(baseline.cases[1].findings[0].kind, "heap-use-after-free");

    let release = "    if (mode == \"release-early\") owner.reset();\n";
    let read = "    std::printf(\"%d\\n\", *borrowed);\n";
    let fixed_source = source.replace(&format!("{release}{read}"), &format!("{read}{release}"));
    assert_ne!(source, fixed_source);
    fs::write(fixture.dir.path().join("main.c"), fixed_source).unwrap();
    let fixed = fixture
        .engine
        .retest(&baseline.id, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(fixed.execution, "complete");
    assert_eq!(fixed.outcome, "no_errors_observed", "{fixed:#?}");
    assert!(fixed.cases.iter().all(|case| case.process.stdout == "42\n"));
    assert_eq!(
        fixed.comparison.as_ref().unwrap().baseline_report_id,
        baseline.id
    );
    let saved = fixture.engine.store.load(&baseline.id).unwrap();
    assert_eq!(saved.cases[1].findings[0].kind, "heap-use-after-free");
}

#[test]
fn failed_build_is_saved_and_runs_no_cases() {
    let fixture = Fixture::new("this is not C\n", vec![case("safe")]);
    let report = fixture.check();
    assert_eq!(report.outcome, "incomplete");
    assert!(!report.build.succeeded());
    assert!(!report.build.stderr.is_empty());
    assert!(report.cases.is_empty());
    fixture.engine.store.load(&report.id).unwrap();
}

#[test]
fn unfinished_background_work_is_incomplete() {
    let fixture = Fixture::new(include_str!("fixtures/c/cases.c"), vec![case("background")]);
    let report = fixture.check();
    assert_eq!(report.cases[0].process.exit_code, Some(0));
    assert_eq!(report.cases[0].status, "descendants_terminated");
    assert_eq!(report.outcome, "incomplete");
    assert_eq!(report.execution, "incomplete");
}

#[test]
fn finishing_helper_is_drained_without_a_false_incomplete_result() {
    let fixture = Fixture::new(include_str!("fixtures/c/cases.c"), vec![case("finishing")]);
    let report = fixture.check();
    assert_eq!(report.cases[0].process.exit_code, Some(0));
    assert_eq!(report.outcome, "no_errors_observed", "{report:#?}");
    assert_eq!(report.execution, "complete");
}

#[test]
fn declared_headers_and_cpp_standard_library_build() {
    let fixture = Fixture::new(
        "#include <iostream>\n#include \"answer.h\"\nint main() { std::cout << answer() << '\\n'; }\n",
        vec![json!({"id":"cpp"})],
    );
    fs::create_dir(fixture.dir.path().join("include")).unwrap();
    fs::write(
        fixture.dir.path().join("include/answer.h"),
        "inline int answer() { return 42; }\n",
    )
    .unwrap();
    fixture.edit_config(|v| {
        let t = &mut v["targets"][0];
        t["language"] = json!("cpp");
        t["headers"] = json!(["include/answer.h"]);
        t["include_dirs"] = json!(["include"]);
    });
    let report = fixture.check();
    assert_eq!(report.outcome, "no_errors_observed", "{report:#?}");
    assert_eq!(report.cases[0].process.stdout, "42\n");
    fixture.engine.store.load(&report.id).unwrap();
}

#[test]
fn snapshot_tampering_and_path_escape_are_rejected() {
    let fixture = Fixture::new("int main(void) { return 0; }\n", vec![case("safe")]);
    let report = fixture.check();
    fs::write(
        fixture
            .engine
            .store
            .root
            .join(&report.id)
            .join("project/main.c"),
        "modified",
    )
    .unwrap();
    assert!(
        fixture
            .engine
            .store
            .load(&report.id)
            .unwrap_err()
            .contains("fingerprint mismatch")
    );
    assert!(
        fixture
            .engine
            .retest(&report.id, &AtomicBool::new(false))
            .is_err()
    );
    assert!(fixture.engine.store.load("../targets.json").is_err());
    fixture.edit_config(|v| v["targets"][0]["sources"] = json!(["../secret.c"]));
    assert!(
        fixture
            .engine
            .check("test", &AtomicBool::new(false))
            .is_err()
    );
}

#[test]
fn invalid_definitions_cannot_start() {
    let fixture = Fixture::new("int main(void) { return 0; }\n", vec![case("safe")]);
    let original: Value = serde_json::from_slice(&fs::read(&fixture.config).unwrap()).unwrap();
    let invalid = [
        ("cases", json!([])),
        ("cases", json!([case("same"), case("same")])),
        ("sources", json!(["main.c", "main.c"])),
        ("extra", json!(true)),
        ("limits", json!({"test_timeout_ms":0})),
        ("limits", json!({"output_bytes":999999999})),
        ("cases", json!([{"id":"nul","args":["a\u{0}b"]}])),
    ];
    for (field, value) in invalid {
        let mut document = original.clone();
        document["targets"][0][field] = value;
        fs::write(&fixture.config, serde_json::to_vec(&document).unwrap()).unwrap();
        assert!(Catalog::load(&fixture.config).is_err(), "{field}");
    }
    assert!(fixture.engine.store.list(0).is_err());
    assert!(fixture.engine.store.list(101).is_err());
    let public = fixture.dir.path().join("public");
    fs::create_dir(&public).unwrap();
    fs::set_permissions(&public, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(Store::open(&public).is_err());
}

#[test]
fn pre_report_errors_have_discoverable_traces_without_sensitive_configuration() {
    let fixture = Fixture::new("int main(void) { return 0; }", vec![case("safe")]);
    fixture.edit_config(|v| v["targets"][0]["sources"] = json!(["missing-private-source.c"]));
    let error = fixture
        .engine
        .check("test", &AtomicBool::new(false))
        .unwrap_err();
    let listed = boundscope::trace::list(&fixture.engine.store, 20).unwrap();
    assert_eq!(listed.len(), 1);
    let id = listed[0]["trace_id"].as_str().unwrap();
    assert!(error.contains(id));
    let trace = boundscope::trace::load(&fixture.engine.store, id).unwrap();
    assert_eq!(trace.state, "failed");
    assert!(matches!(
        trace.events.last().unwrap().detail,
        boundscope::trace::Event::Failed {
            phase: boundscope::trace::Phase::Snapshot,
            cancelled: false,
            ..
        }
    ));
    let serialized = serde_json::to_string(&trace).unwrap();
    assert!(!serialized.contains("missing-private-source"));
    assert!(!serialized.contains(fixture.dir.path().to_str().unwrap()));
    assert!(fixture.engine.store.list(20).unwrap().is_empty());
    assert!(
        fixture
            .engine
            .check("bad\nidentifier", &AtomicBool::new(false))
            .is_err()
    );
    assert_eq!(
        boundscope::trace::list(&fixture.engine.store, 20)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn cancellation_stops_active_processes_and_saves_incomplete_evidence() {
    let fixture = Fixture::new(include_str!("fixtures/c/cases.c"), vec![case("timeout")]);
    fixture.edit_config(|v| v["targets"][0]["limits"]["test_timeout_ms"] = json!(30000));
    let cancel = Arc::new(AtomicBool::new(false));
    let signal = cancel.clone();
    let engine = fixture.engine.clone();
    let started = Instant::now();
    let worker = thread::spawn(move || engine.check("test", &cancel));
    // Wait for compilation to produce the test executable before cancelling.
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        let built = fs::read_dir(&fixture.engine.store.root)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.path().join("program").exists());
        if built {
            thread::sleep(Duration::from_millis(100));
            break;
        }
        assert!(
            Instant::now() < until,
            "compiler did not produce the fixture"
        );
        thread::sleep(Duration::from_millis(20));
    }
    signal.store(true, Ordering::Release);
    let report = worker.join().unwrap().unwrap();
    assert_eq!(report.execution, "cancelled");
    assert_eq!(report.outcome, "incomplete");
    assert!(started.elapsed() < Duration::from_secs(25));
    assert!(
        !fs::read_dir(&fixture.engine.store.root)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().starts_with(".work-"))
    );
}
