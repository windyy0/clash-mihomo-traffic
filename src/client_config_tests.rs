use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn replacing_client_config_retries_after_a_shared_file_handle_is_released() {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.yaml");
    fs::write(&path, "find-process-mode: strict\n").unwrap();
    let open = || {
        fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(&path)
            .unwrap()
    };
    let occupied = std::cell::RefCell::new(Some(open()));
    let replacement = b"find-process-mode: always\n";
    assert!(atomic_write(&path, replacement, || anyhow::bail!("连接已变化")).is_err());
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "find-process-mode: strict\n"
    );
    let retries = std::cell::Cell::new(0);
    atomic_write(&path, replacement, || {
        retries.set(retries.get() + 1);
        occupied.borrow_mut().take();
        Ok(())
    })
    .unwrap();
    assert_eq!(retries.get(), 1);
    assert_eq!(fs::read(&path).unwrap(), replacement);
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

fn target(kind: Kind, root: &Path) -> Target {
    Target {
        kind,
        root: root.to_owned(),
        executable: None,
    }
}
fn verge(root: &Path) -> Target {
    fs::create_dir(root.join("profiles")).unwrap();
    fs::write(
        root.join("config.yaml"),
        "mixed-port: 7890\nexternal-controller-pipe: test-pipe\n",
    )
    .unwrap();
    fs::write(root.join("profiles.yaml"), "current: subscription\nitems:\n- uid: Merge\n  type: merge\n  file: Merge.yaml\n- uid: subscription\n  type: remote\n  url: https://example.invalid/private\n").unwrap();
    target(Kind::Verge, root)
}
fn nyan(root: &Path) -> Target {
    fs::create_dir(root.join("profiles")).unwrap();
    fs::write(root.join("profiles.yaml"), "# keep header\n_nyanpasu:\n  document: profiles\n  schema_revision: 4\nrevision: 8 # keep revision note\ncurrent: subscription\nvalid: [dns, tun] # keep filter note\nglobal_transforms: [custom-script]\nitems:\n  - uid: subscription\n    name: Mine\n    type: config\n    config: { type: file, source: { type: remote, file: sub.yaml, url: 'https://example.invalid/private' } }\n  - uid: custom-script\n    name: Mine\n    type: transform\n    transform: { type: script, runtime: javascript, source: { type: local, binding: { type: managed, file: mine.js } } }\n# final comment stays\nunknown_key: { keep: true }\n").unwrap();
    target(Kind::Nyanpasu, root)
}
fn fl(root: &Path) -> Target {
    let inner = serde_json::json!({"themeProps":{"mode":"dark"},"patchClashConfig":{"find-process-mode":"off","secret":"private","unknown":42},"unknownConfig":{"keep":true}}).to_string();
    let source = format!(
        "{{\n  \"flutter.other\": \"keep exact spacing\",\n  \"flutter.config\": {},\n  \"unknown\" : [1, 2, 3]\n}}\n",
        serde_json::to_string(&inner).unwrap()
    );
    fs::write(root.join("shared_preferences.json"), source).unwrap();
    target(Kind::FlClash, root)
}
#[test]
fn verge_modes_preserve_comments_unknown_fields_and_backup_original_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let target = verge(temp.path());
    let file = temp.path().join("profiles/Merge.yaml");
    let original = "# keep\r\n'find-process-mode': strict # user note\r\nsniffer:\r\n  enable: true\r\nunknown: 'literal # value'\r\n";
    fs::write(&file, original).unwrap();
    for desired in ProcessMode::ALL {
        let before = fs::read(&file).unwrap();
        let backup = target
            .prepare(desired)
            .unwrap()
            .commit(&temp.path().join("backups"), || true)
            .unwrap();
        assert_eq!(target.read().unwrap().mode, SavedMode::Explicit(desired));
        let after = fs::read_to_string(&file).unwrap();
        assert!(after.contains("# user note"));
        assert!(after.contains("sniffer:\r\n  enable: true\r\nunknown: 'literal # value'\r\n"));
        if let Some(backup) = backup {
            assert_eq!(fs::read(backup.join("0.original")).unwrap(), before);
        }
    }
    assert!(target.prepare(ProcessMode::Off).unwrap().edits.is_empty());
}
#[test]
fn unset_and_unknown_verge_mode_are_never_reported_as_strict() {
    let temp = tempfile::tempdir().unwrap();
    let target = verge(temp.path());
    assert_eq!(target.read().unwrap().mode, SavedMode::Inherited);
    fs::write(
        temp.path().join("profiles/Merge.yaml"),
        "find-process-mode: future\n",
    )
    .unwrap();
    assert!(matches!(target.read().unwrap().mode, SavedMode::Unknown(_)));
    fs::write(
        temp.path().join("profiles/Merge.yaml"),
        "find-process-mode: [strict]\n",
    )
    .unwrap();
    assert!(matches!(target.read().unwrap().mode, SavedMode::Unknown(_)));
}
#[test]
fn nyan_creates_one_global_overlay_preserves_chain_filter_and_unknown_data() {
    let temp = tempfile::tempdir().unwrap();
    let target = nyan(temp.path());
    let before = fs::read_to_string(temp.path().join("profiles.yaml")).unwrap();
    assert!(matches!(target.read().unwrap().mode, SavedMode::Unknown(_)));
    for desired in ProcessMode::ALL {
        target
            .prepare(desired)
            .unwrap()
            .commit(&temp.path().join("backups"), || true)
            .unwrap();
        assert_eq!(target.read().unwrap().mode, SavedMode::Explicit(desired));
    }
    let after = fs::read_to_string(temp.path().join("profiles.yaml")).unwrap();
    let value = yaml(&after).unwrap();
    assert_eq!(value["revision"].as_u64(), Some(11));
    assert_eq!(
        value["global_transforms"].as_sequence().unwrap(),
        &vec![
            Yaml::String("custom-script".into()),
            Yaml::String(NYAN_UID.into())
        ]
    );
    assert_eq!(value["valid"].as_sequence().unwrap().len(), 3);
    assert_eq!(value["items"].as_sequence().unwrap().len(), 3);
    assert_eq!(value["items"][0], yaml(&before).unwrap()["items"][0]);
    assert!(after.contains("# keep revision note") && after.contains("# keep filter note"));
    assert!(after.contains("# final comment stays\nunknown_key: { keep: true }\n"));
    assert!(target.prepare(ProcessMode::Off).unwrap().edits.is_empty());
}
#[test]
fn flclash_preserves_other_preferences_exactly_and_other_config_values_semantically() {
    let temp = tempfile::tempdir().unwrap();
    let target = fl(temp.path());
    let before = target.fl_document().unwrap().2;
    for desired in ProcessMode::ALL {
        target
            .prepare(desired)
            .unwrap()
            .commit(&temp.path().join("backups"), || true)
            .unwrap();
        assert_eq!(target.read().unwrap().mode, SavedMode::Explicit(desired));
        let (source, _, after) = target.fl_document().unwrap();
        assert!(source.contains("\"unknown\" : [1, 2, 3]"));
        assert_eq!(after["themeProps"], before["themeProps"]);
        assert_eq!(after["unknownConfig"], before["unknownConfig"]);
        assert_eq!(
            after["patchClashConfig"]["unknown"],
            before["patchClashConfig"]["unknown"]
        );
        assert_eq!(
            after["patchClashConfig"]["secret"],
            before["patchClashConfig"]["secret"]
        );
    }
    assert!(target.prepare(ProcessMode::Off).unwrap().edits.is_empty());
}

#[test]
fn restore_inheritance_preserves_other_settings_backs_up_and_is_idempotent() {
    for kind in [Kind::Verge, Kind::Nyanpasu, Kind::FlClash] {
        let temp = tempfile::tempdir().unwrap();
        let target = match kind {
            Kind::Verge => verge(temp.path()),
            Kind::Nyanpasu => nyan(temp.path()),
            Kind::FlClash => fl(temp.path()),
        };
        target
            .prepare(ProcessMode::Always)
            .unwrap()
            .commit(&temp.path().join("backups"), || true)
            .unwrap();
        let file = target.read().unwrap().source;
        if kind != Kind::FlClash {
            let source = fs::read_to_string(&file).unwrap();
            fs::write(&file, format!("# preserve header\r\n{}\r\n# preserve next field\r\nunknown_restore: {{keep: true}}\r\n", source.trim().replace('\n', "\r\n"))).unwrap();
        }
        let before = fs::read(&file).unwrap();
        let before_json = (kind == Kind::FlClash).then(|| target.fl_document().unwrap().2);
        let metadata_before = (kind == Kind::Nyanpasu).then(|| {
            yaml(&fs::read_to_string(temp.path().join("profiles.yaml")).unwrap()).unwrap()
        });
        let backup = target
            .prepare_restore()
            .unwrap()
            .commit(&temp.path().join("backups"), || true)
            .unwrap()
            .unwrap();
        assert_eq!(fs::read(backup.join("0.original")).unwrap(), before);
        assert_eq!(target.read().unwrap().mode, SavedMode::Inherited);
        if let Some(mut expected) = before_json {
            expected["patchClashConfig"]
                .as_object_mut()
                .unwrap()
                .remove(FIELD);
            let (source, _, actual) = target.fl_document().unwrap();
            assert_eq!(actual, expected);
            assert!(source.contains("\"unknown\" : [1, 2, 3]"));
        } else {
            let after = fs::read_to_string(&file).unwrap();
            assert!(after.contains("# preserve header\r\n"));
            assert!(after.contains("# preserve next field\r\nunknown_restore: {keep: true}\r\n"));
            assert!(yaml(&after).unwrap().get(FIELD).is_none());
        }
        if let Some(mut expected) = metadata_before {
            let actual =
                yaml(&fs::read_to_string(temp.path().join("profiles.yaml")).unwrap()).unwrap();
            expected["revision"] =
                Yaml::Number((expected["revision"].as_u64().unwrap() + 1).into());
            assert_eq!(
                actual, expected,
                "restore changed another Nyanpasu chain or filter"
            );
        }
        assert!(target.prepare_restore().unwrap().edits.is_empty());
    }
}

#[test]
fn restore_detects_concurrent_edits_and_rolls_back_nyan_multi_file_failure() {
    let temp = tempfile::tempdir().unwrap();
    let target = nyan(temp.path());
    target
        .prepare(ProcessMode::Always)
        .unwrap()
        .commit(&temp.path().join("backups"), || true)
        .unwrap();
    let file = target.read().unwrap().source;
    let before = fs::read(&file).unwrap();
    let metadata = fs::read(temp.path().join("profiles.yaml")).unwrap();
    let changes = target.prepare_restore().unwrap();
    let checks = AtomicUsize::new(0);
    assert!(
        changes
            .commit(&temp.path().join("backups"), || checks
                .fetch_add(1, Ordering::Relaxed)
                < 2)
            .is_err()
    );
    assert_eq!(fs::read(&file).unwrap(), before);
    assert_eq!(
        fs::read(temp.path().join("profiles.yaml")).unwrap(),
        metadata
    );
    let changes = target.prepare_restore().unwrap();
    fs::write(&file, "find-process-mode: strict\nexternal: keep\n").unwrap();
    assert!(
        changes
            .commit(&temp.path().join("backups"), || true)
            .is_err()
    );
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        "find-process-mode: strict\nexternal: keep\n"
    );
}

#[test]
fn yaml_restore_keeps_comments_bom_and_rejects_unsafe_representations() {
    let source =
        "\u{feff}'find-process-mode': strict # mode note\r\n# other note\r\nunknown: keep\r\n";
    let after = remove_yaml_field(source, FIELD).unwrap();
    assert_eq!(
        after,
        "\u{feff}# mode note\r\n# other note\r\nunknown: keep\r\n"
    );
    assert_eq!(
        remove_yaml_field("---\nfind-process-mode: off\n...\n", FIELD).unwrap(),
        "---\n...\n"
    );
    assert!(remove_yaml_field("{find-process-mode: strict}", FIELD).is_err());
    assert!(remove_yaml_field("find-process-mode: &a strict\nunknown: *a\n", FIELD).is_err());
    let temp = tempfile::tempdir().unwrap();
    let target = verge(temp.path());
    assert!(target.prepare_restore().unwrap().edits.is_empty());
    assert!(!temp.path().join("profiles/Merge.yaml").exists());
    let temp = tempfile::tempdir().unwrap();
    let target = nyan(temp.path());
    assert!(
        target.prepare_restore().is_err(),
        "must not remove another Nyanpasu global chain"
    );
}
#[test]
fn concurrent_file_and_verge_metadata_edits_abort_without_overwriting() {
    let temp = tempfile::tempdir().unwrap();
    let target = verge(temp.path());
    let path = temp.path().join("profiles/Merge.yaml");
    fs::write(&path, "unknown: original\n").unwrap();
    let changes = target.prepare(ProcessMode::Always).unwrap();
    fs::write(&path, "unknown: external\n").unwrap();
    assert!(
        changes
            .commit(&temp.path().join("backups"), || true)
            .is_err()
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "unknown: external\n");
    let changes = target.prepare(ProcessMode::Always).unwrap();
    fs::write(temp.path().join("profiles.yaml"), "items: []\n").unwrap();
    assert!(
        changes
            .commit(&temp.path().join("backups"), || true)
            .is_err()
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "unknown: external\n");
}
#[test]
fn cancellation_between_nyan_files_rolls_back_the_overlay_and_keeps_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let target = nyan(temp.path());
    let before = fs::read(temp.path().join("profiles.yaml")).unwrap();
    let checks = AtomicUsize::new(0);
    let changes = target.prepare(ProcessMode::Always).unwrap();
    assert!(
        changes
            .commit(&temp.path().join("backups"), || checks
                .fetch_add(1, Ordering::Relaxed)
                < 2)
            .is_err()
    );
    assert_eq!(fs::read(temp.path().join("profiles.yaml")).unwrap(), before);
    assert!(!temp.path().join("profiles").join(NYAN_FILE).exists());
}
#[test]
fn cancellation_does_not_rollback_an_external_change() {
    let temp = tempfile::tempdir().unwrap();
    let target = nyan(temp.path());
    let path = temp.path().join("profiles").join(NYAN_FILE);
    let checks = AtomicUsize::new(0);
    let changes = target.prepare(ProcessMode::Always).unwrap();
    let result = changes.commit(&temp.path().join("backups"), || {
        if checks.fetch_add(1, Ordering::Relaxed) == 2 {
            fs::write(&path, "external: true\n").unwrap();
            false
        } else {
            true
        }
    });
    assert!(result.unwrap_err().to_string().contains("回滚失败"));
    assert_eq!(fs::read_to_string(&path).unwrap(), "external: true\n");
}
#[test]
fn unsafe_paths_unknown_schemas_and_colliding_overlay_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let target = verge(temp.path());
    fs::write(
        temp.path().join("profiles.yaml"),
        "items:\n- uid: Merge\n  type: merge\n  file: ../outside.yaml\n",
    )
    .unwrap();
    assert!(target.prepare(ProcessMode::Always).is_err());
    let temp = tempfile::tempdir().unwrap();
    let target = nyan(temp.path());
    fs::write(
        temp.path().join("profiles").join(NYAN_FILE),
        "unrelated: true\n",
    )
    .unwrap();
    assert!(target.prepare(ProcessMode::Always).is_err());
    let source = fs::read_to_string(temp.path().join("profiles.yaml"))
        .unwrap()
        .replace("schema_revision: 4", "schema_revision: 99");
    fs::write(temp.path().join("profiles.yaml"), source).unwrap();
    assert!(target.read().is_err());
}
#[test]
fn source_editor_handles_root_sequences_crlf_and_rejects_duplicate_yaml_keys() {
    let original = "# header\r\nitems:\r\n- uid: a\r\n  type: config\r\nunknown: keep\r\n";
    let updated = append_yaml_sequence(
        original,
        "items",
        serde_yaml::from_str("uid: b\ntype: transform").unwrap(),
    )
    .unwrap();
    assert!(updated.contains("- uid: a\r\n  type: config\r\n"));
    assert_eq!(
        yaml(&updated).unwrap()["items"]
            .as_sequence()
            .unwrap()
            .len(),
        2
    );
    assert!(
        set_yaml_field(
            "find-process-mode: off\nfind-process-mode: strict\n",
            FIELD,
            &Yaml::String("always".into())
        )
        .is_err()
    );
    assert!(
        set_yaml_field(
            "{ find-process-mode: off }",
            FIELD,
            &Yaml::String("always".into())
        )
        .is_err()
    );
}

#[test]
fn verge_service_alias_matches_only_the_same_release_owner() {
    let owner = "a".repeat(64);
    let sidecar = format!(r"\\.\pipe\verge-mihomo-sidecar-release-{owner}");
    let service = format!(r"\\.\pipe\verge-mihomo-production-{owner}");
    assert_eq!(verge_pipe_key(&sidecar), verge_pipe_key(&service));
    assert_eq!(verge_pipe_key(&sidecar), Some(owner.as_str()));
    assert_ne!(
        verge_pipe_key(&sidecar),
        verge_pipe_key(&format!(
            r"\\.\pipe\verge-mihomo-production-{}",
            "b".repeat(64)
        ))
    );
    assert!(verge_pipe_key(r"\\.\pipe\verge-mihomo-production-short").is_none());
    assert!(verge_pipe_key(&format!(r"\\.\pipe\verge-mihomo-sidecar-dev-{owner}")).is_none());
}

#[test]
fn write_denied_keeps_the_original_and_retains_backup() {
    let temp = tempfile::tempdir().unwrap();
    let target = verge(temp.path());
    let path = temp.path().join("profiles/Merge.yaml");
    let original = b"# original\nunknown: keep\n";
    fs::write(&path, original).unwrap();
    let changes = target.prepare(ProcessMode::Always).unwrap();
    let permissions = fs::metadata(&path).unwrap().permissions();
    let mut readonly = permissions.clone();
    readonly.set_readonly(true);
    fs::set_permissions(&path, readonly).unwrap();
    let result = changes.commit(&temp.path().join("backups"), || true);
    fs::set_permissions(&path, permissions).unwrap();
    assert!(result.is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(
        fs::read_dir(temp.path().join("backups")).unwrap().count(),
        1
    );
}

/// Explicit opt-in only: exercise the real worker against the running Verge,
/// with an isolated ledger and unconditional file/runtime restoration.
#[tokio::test]
#[ignore = "temporarily edits the running Verge's global mode, then restores it"]
async fn live_verge_save_apply_and_restore() -> Result<()> {
    use crate::{
        backend::Backend,
        config::{Config, ProcessControl},
        mihomo::Client,
        permanent_control::{Request, Status},
    };
    use std::time::{Duration, Instant};
    ensure!(
        std::env::var("MIHOMO_TRAFFIC_LIVE_CONFIG_CHECK").as_deref() == Ok("1"),
        "需要显式启用实机验证"
    );
    async fn wait(
        handle: &Backend,
        condition: impl Fn(&crate::model::Shared) -> bool,
    ) -> Result<()> {
        let start = Instant::now();
        loop {
            if condition(&handle.shared.lock().unwrap()) {
                return Ok(());
            }
            if start.elapsed() >= Duration::from_secs(8) {
                let state = handle.shared.lock().unwrap();
                bail!(
                    "实机验证等待超时：connected={} client={} verified={} runtime={} permanent={:?} error={} sample={}",
                    state.connected,
                    state.client_name,
                    state.core_verified,
                    state.process_mode,
                    state.permanent.status,
                    state.permanent.error,
                    state.status
                );
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    let original_config = Config::load(
        &dirs::data_local_dir()
            .context("无本地数据目录")?
            .join("MihomoTraffic"),
    )?;
    ensure!(
        original_config.process_control == ProcessControl::Unmanaged,
        "原实例正在自动保持，请勿在此时执行实机往返验证"
    );
    let data = tempfile::tempdir()?;
    original_config.save(data.path())?;
    let handle = Backend::start(data.path().to_owned())?;
    let initialized = wait(&handle, |s| {
        s.connected
            && s.client_name == "Clash Verge Rev"
            && s.permanent.available
            && ProcessMode::parse(&s.process_mode).is_some()
    })
    .await;
    if let Err(error) = initialized {
        let _ = handle.request_shutdown();
        return Err(error);
    }
    let state = handle.shared.lock().unwrap().clone();
    let source = PathBuf::from(&state.permanent.source);
    let before = read_optional(&source)?;
    let original_mode = ProcessMode::parse(&state.process_mode).unwrap();
    let desired = if original_mode == ProcessMode::Always {
        ProcessMode::Strict
    } else {
        ProcessMode::Always
    };
    let expected_file = set_yaml_field(
        std::str::from_utf8(before.as_deref().unwrap_or_default())?,
        FIELD,
        &Yaml::String(desired.as_str().into()),
    )?
    .into_bytes();
    let result: Result<()> = async {
        handle
            .permanent_control
            .post(&handle, Request::Save(desired));
        wait(&handle, |s| {
            s.permanent.status == Status::Applied && s.process_mode == desired.as_str()
        })
        .await?;
        ensure!(
            handle.config.lock().unwrap().process_control == ProcessControl::Unmanaged,
            "永久保存后未停止保持"
        );
        wait(&handle, |s| s.connected && s.live.len() > state.live.len()).await?;
        ensure!(
            read_optional(&source)?.as_deref() == Some(expected_file.as_slice()),
            "实机文件回读与预期不同"
        );
        Ok(())
    }
    .await;
    let _ = handle.apply_process_mode(ProcessControl::Unmanaged);
    let _ = handle.request_shutdown();
    let start = Instant::now();
    while !handle.done.load(Ordering::Acquire) && start.elapsed() < Duration::from_secs(6) {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let restore_file = (|| -> Result<()> {
        let now = read_optional(&source)?;
        if now == before {
            return Ok(());
        }
        ensure!(
            now.as_deref() == Some(expected_file.as_slice()),
            "配置发生外部修改，未覆盖外部内容；原始备份在 {}",
            data.path().display()
        );
        match &before {
            Some(bytes) => atomic_write(&source, bytes, || Ok(())),
            None => fs::remove_file(&source).map_err(Into::into),
        }
    })();
    let restore_runtime: Result<()> = async {
        let mut config = original_config;
        config.endpoint = state.endpoint;
        let mut client = Client::new(&config)?;
        client.pin_identity(state.core_identity);
        client
            .request_with_body(
                reqwest::Method::PATCH,
                "/configs",
                Some(&serde_json::json!({FIELD:original_mode.as_str()})),
            )
            .await?;
        let readback: serde_json::Value =
            serde_json::from_slice(&client.request("/configs").await?)?;
        ensure!(
            readback[FIELD].as_str() == Some(original_mode.as_str()),
            "实机模式恢复确认失败"
        );
        Ok(())
    }
    .await;
    if restore_file.is_err() || restore_runtime.is_err() {
        let preserved = data.keep();
        eprintln!("验证恢复失败，保留备份目录：{}", preserved.display());
    }
    restore_file?;
    restore_runtime?;
    result?;
    println!(
        "Live Verge: {} -> {}, file restored byte-for-byte, runtime restored to {}, sampling continued in isolated ledger",
        original_mode.as_str(),
        desired.as_str(),
        original_mode.as_str()
    );
    Ok(())
}
