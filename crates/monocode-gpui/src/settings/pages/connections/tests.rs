use super::*;
use std::io::{BufRead, Read, Write};

#[test]
fn failed_revocation_preserves_connection_until_local_only_removal() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let mut methods = vec![];
        for response in [
            json!({"result": {"protocolVersion": 1, "environmentId": "env", "name": "Server"}}),
            json!({"error": "device revocation failed"}),
        ] {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some((key, value)) = line.split_once(':')
                    && key.eq_ignore_ascii_case("content-length")
                {
                    length = value.trim().parse().unwrap();
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let request: Value = serde_json::from_slice(&body).unwrap();
            methods.push(request["method"].as_str().unwrap().to_owned());
            let body = response.to_string();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        }
        methods
    });
    let directory = std::env::temp_dir().join(format!(
        "monocode-revocation-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let remote = Connections::new(&directory);
    let machine = remote.connect(String::new(), url, "a".repeat(43)).unwrap();
    let error = remove_machine(&remote, machine.clone(), true).unwrap_err();
    assert!(error.contains("Server was not removed"));
    assert_eq!(remote.list().unwrap(), vec![machine.clone()]);
    remove_machine(&remote, machine, false).unwrap();
    assert!(remote.list().unwrap().is_empty());
    assert_eq!(
        server.join().unwrap(),
        ["environment.describe", "devices.revokeSelf"]
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn identity_change_never_reports_a_healthy_connection() {
    assert_eq!(
        describe_health(
            "known-host",
            &json!({"environmentId": "other-host", "providers": ["codex"], "capabilities": ["workspace.run", "git.worktreeCreate"]})
        ),
        Err("Host identity changed".into())
    );
}

#[test]
fn health_checks_explorer_and_worktree_support_before_provider_installation() {
    assert_eq!(
        describe_health("host", &json!({"environmentId": "host", "providers": []})).unwrap(),
        Health {
            label: "Connected · host update needed for Explorer and Changes".into(),
            needs_update: true
        }
    );
    assert_eq!(describe_health("host", &json!({"environmentId": "host", "providers": [], "capabilities": ["workspace.run", "git.worktreeCreate"]})).unwrap(), Health {
        label: "Connected · install a supported provider on the host".into(), needs_update: false
    });
}

#[test]
fn ssh_port_uses_config_by_default_and_rejects_invalid_ports() {
    assert_eq!(parse_port(""), Ok(None));
    assert_eq!(parse_port(" 2222 "), Ok(Some(2222)));
    for value in ["0", "65536", "-1", "22.5", "ssh"] {
        assert!(parse_port(value).is_err());
    }
}
