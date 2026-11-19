use super::*;

#[test]
fn parses_single_server_from_standard_json() {
    let (name, server) = server_from_json(
        "claude",
        "",
        r#"{"mcpServers":{"docs":{"command":"npx","args":["server"]}}}"#,
    )
    .unwrap();
    assert_eq!(name, "docs");
    assert_eq!(server["command"], "npx");
    assert!(server_from_json(
        "claude",
        "",
        r#"{"mcpServers":{"one":{"command":"npx"},"two":{"command":"node"}}}"#
    )
    .is_err());
    assert!(server_from_json(
        "claude",
        "different",
        r#"{"mcpServers":{"docs":{"command":"npx"}}}"#
    )
    .is_err());
}

#[test]
fn accepts_opencode_names_and_command_arrays() {
    let (name, server) = server_from_json(
        "opencode",
        "docs server",
        r#"{"type":"local","command":["npx","-y","server"]}"#,
    )
    .unwrap();
    assert_eq!(name, "docs server");
    assert_eq!(server["command"], "npx");
    assert_eq!(server["args"], serde_json::json!(["-y", "server"]));
    assert!(server_from_json("claude", "docs server", r#"{"command":"npx"}"#).is_err());
    assert!(server_from_json("opencode", "docs", r#"{"command":[]}"#).is_err());
}

#[test]
fn writes_opencode_one_config_with_environment() {
    let root =
        std::env::temp_dir().join(format!("monocode-opencode-one-{}", rand::random::<u128>()));
    let path = root.join("opencode.jsonc");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(&path, "{\n // keep this setting\n \"theme\": \"dark\", \"mcp\": {\"existing\": {\"type\": \"remote\", \"url\": \"https://example.com\"},},\n}").unwrap();
    let (_, server) = server_from_json(
        "opencode",
        "docs",
        r#"{"type":"local","command":["npx","-y","docs"],"environment":{"TOKEN":"{env:DOCS_TOKEN}"},"enabled":true}"#,
    ).unwrap();
    write_opencode_server(&path, "docs", server, 1).unwrap();
    let config: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(config["theme"], "dark");
    assert_eq!(config["mcp"]["existing"]["url"], "https://example.com");
    assert_eq!(
        config["mcp"]["docs"]["command"],
        serde_json::json!(["npx", "-y", "docs"])
    );
    assert_eq!(
        config["mcp"]["docs"]["environment"]["TOKEN"],
        "{env:DOCS_TOKEN}"
    );
    assert_eq!(config["mcp"]["docs"]["enabled"], true);
    assert!(write_opencode_server(&path, "docs", serde_json::json!({"command":"npx"}), 1).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn writes_opencode_two_config_and_normalizes_standard_env() {
    let root =
        std::env::temp_dir().join(format!("monocode-opencode-two-{}", rand::random::<u128>()));
    let path = root.join("opencode.json");
    let (_, server) = server_from_json(
        "opencode",
        "docs",
        r#"{"mcpServers":{"docs":{"command":"npx","args":["docs"],"env":{"TOKEN":"{env:DOCS_TOKEN}"},"enabled":false}}}"#,
    ).unwrap();
    write_opencode_server(&path, "docs", server, 2).unwrap();
    let config: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        config["mcp"]["servers"]["docs"]["command"],
        serde_json::json!(["npx", "docs"])
    );
    assert_eq!(
        config["mcp"]["servers"]["docs"]["environment"]["TOKEN"],
        "{env:DOCS_TOKEN}"
    );
    assert_eq!(config["mcp"]["servers"]["docs"]["disabled"], true);
    assert!(config["mcp"]["docs"].is_null());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn adds_opencode_two_server_without_changing_existing_timeouts_or_servers() {
    let root = std::env::temp_dir().join(format!(
        "monocode-opencode-timeout-{}",
        rand::random::<u128>()
    ));
    let path = root.join("opencode.json");
    std::fs::create_dir_all(&root).unwrap();
    let original = serde_json::json!({
        "mcp": {
            "timeout": {"startup": 45000, "catalog": 30000, "execution": 600000},
            "servers": {
                "existing": {"type": "local", "command": ["node", "server.js"]}
            }
        }
    });
    std::fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();

    let result = write_opencode_server(
        &path,
        "docs",
        serde_json::json!({"url": "https://example.com/mcp"}),
        2,
    );
    let config: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    std::fs::remove_dir_all(root).unwrap();

    result.unwrap();
    assert_eq!(config["mcp"]["timeout"], original["mcp"]["timeout"]);
    assert_eq!(
        config["mcp"]["servers"]["existing"],
        original["mcp"]["servers"]["existing"]
    );
    assert_eq!(
        config["mcp"]["servers"]["docs"],
        serde_json::json!({"type": "remote", "url": "https://example.com/mcp"})
    );
}

#[test]
fn rejects_legacy_server_named_timeout_without_changing_config() {
    for server in [
        serde_json::json!({"type": "remote", "url": "https://old.example/mcp"}),
        serde_json::json!({"type": "local", "command": ["node", "server.js"]}),
    ] {
        let root = std::env::temp_dir().join(format!(
            "monocode-opencode-legacy-timeout-{}",
            rand::random::<u128>()
        ));
        let path = root.join("opencode.json");
        std::fs::create_dir_all(&root).unwrap();
        let original = serde_json::to_vec(&serde_json::json!({
            "mcp": {"timeout": server}
        }))
        .unwrap();
        std::fs::write(&path, &original).unwrap();

        let result = write_opencode_server(
            &path,
            "timeout",
            serde_json::json!({"url": "https://new.example/mcp"}),
            2,
        );
        let contents = std::fs::read(&path).unwrap();
        std::fs::remove_dir_all(root).unwrap();

        assert_eq!(
            result,
            Err("Existing OpenCode config uses the 1.x MCP layout".into())
        );
        assert_eq!(contents, original);
    }
}

#[test]
fn adds_opencode_two_server_with_timeouts_and_no_existing_servers_map() {
    for timeout in [
        serde_json::json!({"startup": 45000, "catalog": 30000, "execution": 600000}),
        serde_json::json!({"catalog": 30000.0}),
        serde_json::json!({}),
    ] {
        let root = std::env::temp_dir().join(format!(
            "monocode-opencode-timeout-only-{}",
            rand::random::<u128>()
        ));
        let path = root.join("opencode.json");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({"mcp": {"timeout": timeout}})).unwrap(),
        )
        .unwrap();

        let result = write_opencode_server(
            &path,
            "docs",
            serde_json::json!({"url": "https://example.com/mcp"}),
            2,
        );
        let config: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        std::fs::remove_dir_all(root).unwrap();

        result.unwrap();
        assert_eq!(config["mcp"]["timeout"], timeout);
        assert_eq!(
            config["mcp"]["servers"]["docs"],
            serde_json::json!({"type": "remote", "url": "https://example.com/mcp"})
        );
    }
}

#[test]
fn chooses_existing_opencode_jsonc_and_custom_user_config() {
    let root =
        std::env::temp_dir().join(format!("monocode-opencode-path-{}", rand::random::<u128>()));
    let home = root.join("home");
    let project = root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("opencode.jsonc"), "{}").unwrap();
    assert_eq!(
        opencode_config_path(&home, &project, "project", None),
        project.join("opencode.jsonc")
    );
    std::fs::remove_file(project.join("opencode.jsonc")).unwrap();
    std::fs::create_dir_all(project.join(".opencode")).unwrap();
    std::fs::write(project.join(".opencode/opencode.jsonc"), "{}").unwrap();
    assert_eq!(
        opencode_config_path(&home, &project, "project", None),
        project.join(".opencode/opencode.jsonc")
    );
    let custom = root.join("custom.jsonc");
    assert_eq!(
        opencode_config_path(&home, &project, "user", Some(&custom)),
        custom
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn concurrent_additions_keep_every_server() {
    let root = std::env::temp_dir().join(format!("monocode-mcp-lock-{}", rand::random::<u128>()));
    let path = root.join(".cursor/mcp.json");
    std::thread::scope(|scope| {
        for index in 0..8 {
            let path = path.clone();
            scope.spawn(move || {
                write_json_server(
                    &path,
                    &format!("server-{index}"),
                    serde_json::json!({"command":"npx"}),
                )
                .unwrap();
            });
        }
    });
    let value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(value["mcpServers"].as_object().unwrap().len(), 8);
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn replacing_config_restricts_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let root = std::env::temp_dir().join(format!("monocode-mcp-mode-{}", rand::random::<u128>()));
    let path = root.join("mcp.json");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(&path, r#"{"mcpServers":{}}"#).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    write_json_server(&path, "new", serde_json::json!({"command":"npx"})).unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn writes_server_without_discarding_other_configuration() {
    let root = std::env::temp_dir().join(format!("monocode-mcp-write-{}", rand::random::<u128>()));
    let path = root.join(".cursor/mcp.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        r#"{"otherSetting":true,"mcpServers":{"existing":{"command":"node"}}}"#,
    )
    .unwrap();
    write_json_server(&path, "new", serde_json::json!({"command":"npx"})).unwrap();
    let value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(value["otherSetting"], true);
    assert_eq!(value["mcpServers"]["existing"]["command"], "node");
    assert_eq!(value["mcpServers"]["new"]["command"], "npx");
    assert!(write_json_server(&path, "new", serde_json::json!({"command":"npx"})).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn adds_to_existing_cursor_jsonc() {
    let root = std::env::temp_dir().join(format!("monocode-mcp-jsonc-{}", rand::random::<u128>()));
    let path = root.join(".cursor/mcp.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        "{\n // existing server\n \"mcpServers\": {\"old\": {\"command\": \"node\",},},\n}",
    )
    .unwrap();
    write_json_server(&path, "new", serde_json::json!({"command":"npx"})).unwrap();
    let value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(value["mcpServers"]["old"]["command"], "node");
    assert_eq!(value["mcpServers"]["new"]["command"], "npx");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn jsonc_preserves_urls_and_removes_comments_and_trailing_commas() {
    let input = r#"{"mcp":{"servers":{"docs":{"url":"https://example.com/mcp",},},}, // comment
        }"#;
    let value: Value = serde_json::from_str(&strip_jsonc(input)).unwrap();
    assert_eq!(
        value["mcp"]["servers"]["docs"]["url"],
        "https://example.com/mcp"
    );
}

#[test]
fn discovers_provider_configs_without_exposing_credentials() {
    let root = std::env::temp_dir().join(format!("monocode-mcp-{}", rand::random::<u128>()));
    let home = root.join("home");
    let project = root.join("project");
    std::fs::create_dir_all(home.join(".cursor")).unwrap();
    std::fs::create_dir_all(home.join(".codex")).unwrap();
    std::fs::create_dir_all(home.join(".config/opencode")).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    // Stop the ancestor walk inside the fixture so real user configs (which
    // live in ancestors of the temp dir on a dev machine) are never read.
    std::fs::create_dir_all(root.join(".git")).unwrap();
    std::fs::write(home.join(".claude.json"), r#"{"mcpServers":{"one":{"type":"http","url":"https://example.com","headers":{"Authorization":"secret"}}}}"#).unwrap();
    std::fs::write(
        home.join(".cursor/mcp.json"),
        r#"{"mcpServers":{"two":{"command":"npx"}}}"#,
    )
    .unwrap();
    let desktop = root.join("claude_desktop_config.json");
    std::fs::create_dir_all(desktop.parent().unwrap()).unwrap();
    std::fs::write(&desktop, r#"{"mcpServers":{"desktop":{"command":"npx"}}}"#).unwrap();
    std::fs::write(
        home.join(".codex/config.toml"),
        "[mcp_servers.three]\nurl = 'https://example.com'\n",
    )
    .unwrap();
    let codex_raw = std::fs::read_to_string(home.join(".codex/config.toml")).unwrap();
    let codex_config: toml::Value = toml::from_str(&codex_raw).unwrap();
    assert!(codex_config.get("mcp_servers").is_some());
    std::fs::write(home.join(".config/opencode/opencode.jsonc"), "{\"mcp\": {\"servers\": {\"four\": {\"type\": \"remote\", \"url\": \"https://example.com\",},},}}").unwrap();
    let found = discover(&home, &project, None, &desktop, None);
    let names: Vec<_> = found
        .iter()
        .map(|entry| (entry.provider.as_str(), entry.name.as_str()))
        .collect();
    assert_eq!(
        names,
        [
            ("claude", "one"),
            ("claude_desktop", "desktop"),
            ("codex", "three"),
            ("cursor", "two"),
            ("opencode", "four")
        ]
    );
    assert!(!serde_json::to_string(&found).unwrap().contains("secret"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn discovery_preserves_disabled_servers_across_opencode_versions() {
    for servers in [
        serde_json::json!({
            "off": {"type":"local", "command":["node"], "enabled":false},
            "on": {"type":"local", "command":["node"]}
        }),
        serde_json::json!({"servers": {
            "off": {"type":"local", "command":["node"], "disabled":true},
            "on": {"type":"local", "command":["node"], "disabled":false}
        }}),
    ] {
        let mut found = Vec::new();
        add_json_servers(
            &mut found,
            "opencode",
            "project",
            Path::new("opencode.json"),
            Some(&servers),
        );
        assert_eq!(found.len(), 2);
        assert!(
            !found
                .iter()
                .find(|server| server.name == "off")
                .unwrap()
                .enabled
        );
        assert!(
            found
                .iter()
                .find(|server| server.name == "on")
                .unwrap()
                .enabled
        );
    }
}

#[test]
fn discovery_does_not_treat_opencode_timeouts_as_servers() {
    for timeouts in [
        serde_json::json!({}),
        serde_json::json!({"startup":45000}),
        serde_json::json!({"startup":45000,"catalog":60000,"execution":60000}),
    ] {
        let mut found = Vec::new();
        add_json_servers(
            &mut found,
            "opencode",
            "project",
            Path::new("opencode.json"),
            Some(&serde_json::json!({"timeout":timeouts})),
        );
        assert!(found.is_empty());
    }
    let mut found = Vec::new();
    add_json_servers(
        &mut found,
        "opencode",
        "project",
        Path::new("opencode.json"),
        Some(&serde_json::json!({"timeout":{"type":"local","command":["node"]}})),
    );
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "timeout");
}

#[test]
fn discovery_preserves_codex_enablement() {
    let root =
        std::env::temp_dir().join(format!("monocode-mcp-enabled-{}", rand::random::<u128>()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("config.toml");
    std::fs::write(
        &path,
        "[mcp_servers.off]\ncommand = 'node'\nenabled = false\n[mcp_servers.on]\ncommand = 'node'\n",
    ).unwrap();
    let mut found = Vec::new();
    add_toml_file(&mut found, "codex", "user", &path);
    assert_eq!(found.len(), 2);
    assert!(
        !found
            .iter()
            .find(|server| server.name == "off")
            .unwrap()
            .enabled
    );
    assert!(
        found
            .iter()
            .find(|server| server.name == "on")
            .unwrap()
            .enabled
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn does_not_duplicate_home_configs_or_hide_a_server_named_servers() {
    let root = std::env::temp_dir().join(format!("monocode-mcp-{}", rand::random::<u128>()));
    let project = root.join("work/notes");
    std::fs::create_dir_all(root.join(".cursor")).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        root.join(".cursor/mcp.json"),
        r#"{"mcpServers":{"servers":{"command":"npx"},"docs":{"command":"node"}}}"#,
    )
    .unwrap();
    let found = discover(&root, &project, None, &root.join("desktop.json"), None);
    assert_eq!(found.len(), 2);
    assert!(found.iter().all(|server| server.scope == "user"));
    assert!(found.iter().any(|server| server.name == "servers"));
    std::fs::remove_dir_all(root).unwrap();
}
