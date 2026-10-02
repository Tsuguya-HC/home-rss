use home_rss_repo_checks::diff_coverage::{
    ChangedLine, ClassifiedLine, CoverageClass, UncoveredLine, classify_line, classify_report,
    lcov_path_matches, parse_changed_lines, parse_uncovered_lines, render_report, run_with_root,
};

const PLAIN_FN: &str = "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n";

const HTTP_SERVICE_FN: &str = concat!(
    "use spin_sdk::http::Request;\n",
    "use spin_sdk::http_service;\n",
    "\n",
    "#[http_service]\n",
    "async fn handle(_req: Request) -> &'static str {\n",
    "    \"ok\"\n",
    "}\n",
);

#[test]
fn parses_added_lines_with_new_file_positions() {
    // Catches ignoring the hunk header and using old-file line numbers.
    let diff = concat!(
        "diff --git a/server/src/lib.rs b/server/src/lib.rs\n",
        "index abc..def 100644\n",
        "--- a/server/src/lib.rs\n",
        "+++ b/server/src/lib.rs\n",
        "@@ -1,3 +10,4 @@\n",
        " ctx\n",
        "+added\n",
        "-removed\n",
        "+added2\n",
        " unchanged\n",
        "diff --git a/ui/src/api.ts b/ui/src/api.ts\n",
        "index abc..def 100644\n",
        "--- a/ui/src/api.ts\n",
        "+++ b/ui/src/api.ts\n",
        "@@ -4,2 +4,3 @@\n",
        " ctx\n",
        "+fresh\n",
        " tail\n",
    );
    assert_eq!(
        parse_changed_lines(diff),
        vec![
            ChangedLine {
                path: "server/src/lib.rs".to_string(),
                line: 11,
            },
            ChangedLine {
                path: "server/src/lib.rs".to_string(),
                line: 12,
            },
            ChangedLine {
                path: "ui/src/api.ts".to_string(),
                line: 5,
            },
        ]
    );
}

#[test]
fn parses_new_file_diff_from_dev_null() {
    // Catches dropping files whose old side is /dev/null.
    let diff = concat!(
        "diff --git a/new.rs b/new.rs\n",
        "new file mode 100644\n",
        "index 0000000..abc\n",
        "--- /dev/null\n",
        "+++ b/new.rs\n",
        "@@ -0,0 +1,2 @@\n",
        "+one\n",
        "+two\n",
    );
    assert_eq!(
        parse_changed_lines(diff),
        vec![
            ChangedLine {
                path: "new.rs".to_string(),
                line: 1,
            },
            ChangedLine {
                path: "new.rs".to_string(),
                line: 2,
            },
        ]
    );
}

#[test]
fn parses_only_unexecuted_lcov_entries() {
    // Catches reporting executed (count > 0) lines or choking on FN records.
    let lcov = concat!(
        "TN:\n",
        "SF:server/src/lib.rs\n",
        "FN:11,add\n",
        "FNF:1\n",
        "FNH:1\n",
        "DA:11,0\n",
        "DA:12,3\n",
        "DA:13,0\n",
        "end_of_record\n",
        "SF:/abs/repo/server/src/other.rs\n",
        "DA:5,0\n",
        "end_of_record\n",
    );
    assert_eq!(
        parse_uncovered_lines(lcov),
        vec![
            UncoveredLine {
                path: "server/src/lib.rs".to_string(),
                line: 11,
            },
            UncoveredLine {
                path: "server/src/lib.rs".to_string(),
                line: 13,
            },
            UncoveredLine {
                path: "/abs/repo/server/src/other.rs".to_string(),
                line: 5,
            },
        ]
    );
}

#[test]
fn matches_lcov_paths_across_tool_forms() {
    // cargo llvm-cov writes repo-relative or absolute paths, vitest writes
    // paths relative to ui/.
    assert!(lcov_path_matches("server/src/lib.rs", "server/src/lib.rs"));
    assert!(lcov_path_matches(
        "/abs/repo/server/src/lib.rs",
        "server/src/lib.rs"
    ));
    assert!(lcov_path_matches("src/api.ts", "ui/src/api.ts"));
    assert!(!lcov_path_matches(
        "server/src/lib.rs",
        "server/src/other.rs"
    ));
    assert!(!lcov_path_matches("other/src/api.ts", "ui/src/api.ts"));
}

#[test]
fn marks_http_service_bodies_e2e_only() {
    // Catches classifying by body contents while ignoring the attribute.
    assert_eq!(classify_line(HTTP_SERVICE_FN, 6), CoverageClass::E2eOnly);
}

#[test]
fn marks_qualified_spin_request_param_e2e_only() {
    // Catches missing the fully qualified spin_sdk::http::Request form.
    let source = concat!(
        "async fn handle(_req: spin_sdk::http::Request) -> &'static str {\n",
        "    \"ok\"\n",
        "}\n",
    );
    assert_eq!(classify_line(source, 2), CoverageClass::E2eOnly);
}

#[test]
fn marks_imported_spin_request_param_e2e_only() {
    // Catches only matching the qualified path and ignoring `use` imports.
    assert_eq!(classify_line(HTTP_SERVICE_FN, 6), CoverageClass::E2eOnly);
}

#[test]
fn marks_spin_variables_calls_e2e_only() {
    // Catches missing spin_sdk::variables as a Spin-runtime API.
    let source = concat!(
        "async fn run() -> String {\n",
        "    spin_sdk::variables::get(\"retention_days\").unwrap_or_default()\n",
        "}\n",
    );
    assert_eq!(classify_line(source, 2), CoverageClass::E2eOnly);
}

#[test]
fn marks_spin_pg_calls_e2e_only() {
    // Catches missing Connection::open resolved through a spin_sdk::pg import.
    let source = concat!(
        "use spin_sdk::pg::Connection;\n",
        "\n",
        "async fn open_db(address: &str) -> bool {\n",
        "    Connection::open(address).await.is_ok()\n",
        "}\n",
    );
    assert_eq!(classify_line(source, 4), CoverageClass::E2eOnly);
}

#[test]
fn marks_outbound_http_send_e2e_only() {
    // Catches missing outbound HTTP as a Spin-runtime API.
    let source = concat!(
        "async fn fetch(url: &str) -> bool {\n",
        "    spin_sdk::http::send(url).await.is_ok()\n",
        "}\n",
    );
    assert_eq!(classify_line(source, 2), CoverageClass::E2eOnly);
}

#[test]
fn marks_plain_functions_untested() {
    assert_eq!(classify_line(PLAIN_FN, 2), CoverageClass::Untested);
}

#[test]
fn ignores_local_pg_module_that_is_not_spin() {
    // Catches matching any `pg::Connection` without resolving the import.
    let source = concat!(
        "mod pg {\n",
        "    pub struct Connection;\n",
        "    impl Connection {\n",
        "        pub async fn open(_a: &str) -> bool {\n",
        "            true\n",
        "        }\n",
        "    }\n",
        "}\n",
        "\n",
        "async fn open_db(address: &str) -> bool {\n",
        "    pg::Connection::open(address).await\n",
        "}\n",
    );
    assert_eq!(classify_line(source, 11), CoverageClass::Untested);
}

#[test]
fn tracks_function_ranges_through_braces_in_strings() {
    // Hand-written bracket counting without string awareness ends `plain`
    // early at the `"{"` literal and misattributes the lines below it.
    let source = concat!(
        "fn plain() -> &'static str {\n",
        "    let s = \"{\";\n",
        "    s\n",
        "}\n",
        "\n",
        "#[http_service]\n",
        "async fn handle(_req: spin_sdk::http::Request) -> &'static str {\n",
        "    let y = 2;\n",
        "    y.to_string().leak()\n",
        "}\n",
    );
    assert_eq!(classify_line(source, 8), CoverageClass::E2eOnly);
    assert_eq!(classify_line(source, 2), CoverageClass::Untested);
}

#[test]
fn ignores_spin_paths_inside_string_literals() {
    // Substring matching flags the literal; a real parser sees no call.
    let source = concat!(
        "fn plain() -> &'static str {\n",
        "    let s = \"spin_sdk::variables::get\";\n",
        "    s\n",
        "}\n",
    );
    assert_eq!(classify_line(source, 2), CoverageClass::Untested);
}

fn report_of(
    changed: &[ChangedLine],
    uncovered: &[UncoveredLine],
    load: &dyn Fn(&str) -> Option<String>,
) -> Vec<ClassifiedLine> {
    classify_report(changed, uncovered, load)
}

#[test]
fn report_keeps_only_changed_and_uncovered_rust_lines() {
    // Catches emitting covered lines or lines the diff never touched.
    let changed = vec![
        ChangedLine {
            path: "server/src/lib.rs".to_string(),
            line: 11,
        },
        ChangedLine {
            path: "server/src/lib.rs".to_string(),
            line: 12,
        },
    ];
    let uncovered = vec![UncoveredLine {
        path: "server/src/lib.rs".to_string(),
        line: 11,
    }];
    let got = report_of(&changed, &uncovered, &|_| Some(PLAIN_FN.to_string()));
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].path, "server/src/lib.rs");
    assert_eq!(got[0].line, 11);
    assert_eq!(got[0].class, CoverageClass::Untested);
}

#[test]
fn report_never_marks_ui_lines_e2e_only() {
    // UI lines are only ever untested, even when the source mentions Spin paths.
    let changed = vec![ChangedLine {
        path: "ui/src/api.ts".to_string(),
        line: 5,
    }];
    let uncovered = vec![UncoveredLine {
        path: "src/api.ts".to_string(),
        line: 5,
    }];
    let got = report_of(&changed, &uncovered, &|_| {
        Some("spin_sdk::variables::get(\"x\")".to_string())
    });
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].class, CoverageClass::Untested);
}

#[test]
fn report_treats_unreadable_sources_as_untested() {
    // A source that cannot be loaded carries no Spin evidence either way.
    let changed = vec![ChangedLine {
        path: "server/src/lib.rs".to_string(),
        line: 11,
    }];
    let uncovered = vec![UncoveredLine {
        path: "server/src/lib.rs".to_string(),
        line: 11,
    }];
    let got = report_of(&changed, &uncovered, &|_| None);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].class, CoverageClass::Untested);
}

#[test]
fn renders_sorted_tab_separated_report() {
    let lines = vec![
        ClassifiedLine {
            path: "ui/src/api.ts".to_string(),
            line: 5,
            class: CoverageClass::Untested,
        },
        ClassifiedLine {
            path: "handlers.rs".to_string(),
            line: 3,
            class: CoverageClass::E2eOnly,
        },
    ];
    assert_eq!(
        render_report(&lines),
        "handlers.rs:3\te2e-only\nui/src/api.ts:5\tuntested\n"
    );
}

#[test]
fn end_to_end_classifies_through_files() {
    // Catches wiring gaps between parsing, path matching and classification.
    let root = std::env::temp_dir().join(format!("diffcov-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let source = concat!(
        "#[http_service]\n",
        "async fn handle(_req: spin_sdk::http::Request) -> &'static str {\n",
        "    \"ok\"\n",
        "}\n",
        "\n",
        "fn helper(a: i32) -> i32 {\n",
        "    a + 1\n",
        "}\n",
    );
    std::fs::write(root.join("handlers.rs"), source).unwrap();
    std::fs::write(
        root.join("test.diff"),
        concat!(
            "diff --git a/handlers.rs b/handlers.rs\n",
            "index abc..def 100644\n",
            "--- a/handlers.rs\n",
            "+++ b/handlers.rs\n",
            "@@ -1,8 +1,8 @@\n",
            " #[http_service]\n",
            " async fn handle(_req: spin_sdk::http::Request) -> &'static str {\n",
            "-    \"old\"\n",
            "+    \"ok\"\n",
            " }\n",
            " \n",
            " fn helper(a: i32) -> i32 {\n",
            "-    a\n",
            "+    a + 1\n",
            " }\n",
        ),
    )
    .unwrap();
    std::fs::write(
        root.join("rust.lcov"),
        "TN:\nSF:handlers.rs\nDA:3,0\nDA:7,0\nDA:2,5\nend_of_record\n",
    )
    .unwrap();
    std::fs::write(root.join("ui.lcov"), "TN:\nend_of_record\n").unwrap();

    let report = run_with_root(
        root.join("test.diff").to_str().unwrap(),
        root.join("rust.lcov").to_str().unwrap(),
        root.join("ui.lcov").to_str().unwrap(),
        root.to_str().unwrap(),
    );
    std::fs::remove_dir_all(&root).ok();
    assert_eq!(report, "handlers.rs:3\te2e-only\nhandlers.rs:7\tuntested\n");
}

#[test]
fn binary_delegates_to_run_with_argv_paths() {
    // Catches the bin ignoring argv or printing something other than run's output.
    let root = std::env::temp_dir().join(format!("diffcov-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("empty.diff"), "").unwrap();
    std::fs::write(root.join("empty.lcov"), "TN:\nend_of_record\n").unwrap();
    let bin = env!("CARGO_BIN_EXE_diff-coverage");
    let output = std::process::Command::new(bin)
        .arg(root.join("empty.diff"))
        .arg(root.join("empty.lcov"))
        .arg(root.join("empty.lcov"))
        .output()
        .unwrap();
    std::fs::remove_dir_all(&root).ok();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "");
}
