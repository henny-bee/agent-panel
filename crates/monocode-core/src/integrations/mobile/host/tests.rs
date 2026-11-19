use super::*;

fn fixture() -> (
    MobileHost,
    async_channel::Receiver<Event>,
    Identity,
    Identity,
) {
    let join = Identity::generate().unwrap();
    let phone = Identity::generate().unwrap();
    let key = join.fingerprint().unwrap();
    let dir = std::env::temp_dir().join(format!("monocode-mobile-test-{}", uuid::Uuid::new_v4()));
    let (host, events) = MobileHost::open(dir.join("mobile-access.json")).unwrap();
    host.gate.allow(key.clone(), Some(now() + 900));
    {
        let mut inner = host.inner.lock().unwrap();
        inner.config.enabled = true;
        inner.identity = Some(Identity::generate().unwrap());
        inner.invitations.insert(
            key,
            Invitation {
                expires: now() + 900,
            },
        );
    }
    (host, events, join, phone)
}

fn check_pending_start_stays_stopped(shutdown: bool) {
    let (host, _events, _join, _phone) = fixture();
    host.inner.lock().unwrap().config.open_router = false;
    let loading = host.identity_load.lock().unwrap();
    let starting = host.clone();
    let (entered, waiting) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        entered.send(()).unwrap();
        starting.start()
    });
    waiting.recv_timeout(Duration::from_secs(5)).unwrap();
    if shutdown {
        host.shutdown();
    } else {
        host.configure(false, false, "").unwrap();
    }
    drop(loading);
    let result = worker.join().unwrap();
    let listening = host.status().listening;
    host.shutdown();
    if host.path.exists() {
        std::fs::remove_dir_all(host.path.parent().unwrap()).unwrap();
    }
    result.unwrap();
    assert!(!listening, "listener started after shutdown={shutdown}");
}

#[test]
fn pending_start_does_not_listen_after_disable() {
    check_pending_start_stays_stopped(false);
}

#[test]
fn pending_start_does_not_listen_after_shutdown() {
    check_pending_start_stays_stopped(true);
}

#[test]
fn invitation_is_consumed_once_and_only_public_device_metadata_is_persisted() {
    let (host, _events, join, phone) = fixture();
    let key = join.fingerprint().unwrap();
    let params =
        json!({"certificate":phone.certificate,"proof":phone.proof(&key).unwrap(),"name":"iPhone"});
    let mut inner = host.inner.lock().unwrap();
    let result = host.enroll(&mut inner, &key, &params).unwrap();
    assert!(result["deviceId"].is_string());
    assert!(!host.gate.admits(&key));
    assert!(host.gate.admits(&phone.fingerprint().unwrap()));
    assert!(host.enroll(&mut inner, &key, &params).is_err());
    let saved = std::fs::read_to_string(&host.path).unwrap();
    assert!(!saved.contains("privateKey") && !saved.contains(&phone.private_key));
    assert_eq!(inner.config.devices.len(), 1);
    std::fs::remove_dir_all(host.path.parent().unwrap()).unwrap();
}

#[test]
fn invalid_proof_or_expiration_does_not_consume_an_invitation_or_enroll_a_device() {
    let (host, _events, join, phone) = fixture();
    let key = join.fingerprint().unwrap();
    let mut inner = host.inner.lock().unwrap();
    let params = json!({"certificate":phone.certificate,"proof":phone.proof("different-invitation").unwrap()});
    assert!(host.enroll(&mut inner, &key, &params).is_err());
    assert!(inner.config.devices.is_empty() && inner.invitations.contains_key(&key));
    inner.invitations.get_mut(&key).unwrap().expires = now() - 1;
    let params = json!({"certificate":phone.certificate,"proof":phone.proof(&key).unwrap()});
    assert!(host.enroll(&mut inner, &key, &params).is_err());
    assert!(inner.config.devices.is_empty() && !host.path.exists());
}

fn paired() -> (MobileHost, async_channel::Receiver<Event>, String) {
    let (host, events, join, phone) = fixture();
    let key = join.fingerprint().unwrap();
    let params = json!({"certificate":phone.certificate,"proof":phone.proof(&key).unwrap()});
    host.enroll(&mut host.inner.lock().unwrap(), &key, &params)
        .unwrap();
    (host, events, phone.fingerprint().unwrap())
}

#[tokio::test]
async fn usage_continuation_commands_reach_the_desktop_with_their_state_tokens() {
    let (host, events, peer) = paired();
    let answering = tokio::spawn(async move {
        for method in ["session.resumeUsage", "session.resumeUsageAtReset"] {
            let Ok(Event::Request(request)) = events.recv().await else {
                panic!("expected a request");
            };
            assert_eq!(request.method, method);
            assert_eq!(request.params["token"], "paused");
            request.reply(Ok(json!({"sessionId":"s"})));
        }
    });
    for method in ["session.resumeUsage", "session.resumeUsageAtReset"] {
        let result = host.handle(&peer, json!({"version":VERSION,"id":method,"method":method,"params":{"conversationId":"s", "token":"paused", "enabled":true}})).await.unwrap();
        assert_eq!(result["sessionId"], "s");
    }
    answering.await.unwrap();
}

#[tokio::test]
async fn requests_reach_the_app_scoped_to_the_device_and_return_its_answer() {
    let (host, events, peer) = paired();
    let answering = tokio::spawn(async move {
        let Ok(Event::Request(request)) = events.recv().await else {
            panic!("expected a request");
        };
        assert!(request.command_id.ends_with(":c1"));
        assert_eq!(request.method, "session.cancel");
        request.reply(Ok(json!({"sessionId":"s"})));
    });
    let result = host
        .handle(&peer, json!({"version":VERSION,"id":"c1","method":"session.cancel","params":{"conversationId":"s"}}))
        .await
        .unwrap();
    assert_eq!(result["sessionId"], "s");
    answering.await.unwrap();
    assert!(host
        .handle(
            &peer,
            json!({"version":VERSION,"id":"c2","method":"shell.exec","params":{}})
        )
        .await
        .is_err());
    assert!(host
        .handle(
            "stranger",
            json!({"version":VERSION,"id":"c3","method":"workspace.list","params":{}})
        )
        .await
        .is_err());
    std::fs::remove_dir_all(host.path.parent().unwrap()).unwrap();
}

#[tokio::test]
async fn the_catalog_answers_workspace_polls_without_the_app() {
    let (host, _events, peer) = paired();
    host.publish(
        json!({"projects":[{"id":"p"}],"monos":[],"sessions":[{"id":"s","projectId":"p"}]}),
    )
    .unwrap();
    let catalog = host
        .handle(
            &peer,
            json!({"version":VERSION,"id":"c","method":"workspace.list","params":{}}),
        )
        .await
        .unwrap();
    assert_eq!(catalog["sessions"][0]["id"], "s");
    assert!(catalog["addresses"].is_array());
    assert!(host.publish(json!({"projects":[]})).is_err());
    std::fs::remove_dir_all(host.path.parent().unwrap()).unwrap();
}

#[tokio::test]
async fn a_phone_registers_its_push_token_and_turns_categories_off() {
    let (host, _events, peer) = paired();
    let register = |params: Value| {
        host.handle(
            &peer,
            json!({"version":VERSION,"id":"p","method":"device.push","params":params}),
        )
    };
    assert!(register(json!({"token":"https://example.com"}))
        .await
        .is_err());
    register(json!({"token":"ExponentPushToken[abc]","disabled":["agentFinished"]}))
        .await
        .unwrap();
    assert_eq!(host.push_tokens("agentInput"), ["ExponentPushToken[abc]"]);
    assert!(host.push_tokens("agentFinished").is_empty());
    let saved = std::fs::read_to_string(&host.path).unwrap();
    assert!(saved.contains("ExponentPushToken[abc]"));
    host.forget_push_tokens(&["ExponentPushToken[abc]".into()])
        .unwrap();
    assert!(host.push_tokens("agentInput").is_empty());
    std::fs::remove_dir_all(host.path.parent().unwrap()).unwrap();
}
