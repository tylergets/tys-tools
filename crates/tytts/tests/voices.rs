use std::fs;
use std::process::{Command, Output};

fn has_voice(list: &str, name: &str, engine: &str) -> bool {
    list.lines().any(|line| {
        let mut columns = line.split_whitespace();
        columns.next() == Some(name) && columns.next() == Some(engine)
    })
}

#[test]
fn catalog_and_failed_clone_work_offline_without_publishing_partial_voices() {
    let root = std::env::temp_dir().join(format!("tytts-voices-test-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());
    let config = root.join("config.yaml");
    fs::write(&config, "auto_download: false\n").unwrap();
    let invoke = |args: &[&str], locale: &[(&str, &str)]| -> Output {
        Command::new(env!("CARGO_BIN_EXE_tytts"))
            .env("XDG_DATA_HOME", root.join("data"))
            .env("LC_ALL", "")
            .env("LC_MESSAGES", "")
            .env("LANG", "C")
            .envs(locale.iter().copied())
            .arg("--config")
            .arg(&config)
            .args(args)
            .output()
            .unwrap()
    };
    let run = |args: &[&str]| invoke(args, &[]);
    let list = run(&["--voices"]);
    assert!(list.status.success());
    let list = String::from_utf8(list.stdout).unwrap();
    assert!(has_voice(&list, "alba", "pocket"));
    assert!(has_voice(&list, "savannah", "zonos"));
    assert!(has_voice(&list, "qwen-aiden", "qwen"));
    assert!(!has_voice(&list, "qwen-sohee", "qwen"));
    assert!(!has_voice(&list, "bf_emma", "kokoro"));
    assert!(!has_voice(&list, "ff_siwis", "kokoro"));
    for (locale, args, included, excluded) in [
        (
            vec![("LANG", "en_US.UTF-8")],
            vec!["--voices"],
            "af_heart",
            "bf_emma",
        ),
        (
            vec![("LANG", "fr_FR.UTF-8")],
            vec!["--voices"],
            "ff_siwis",
            "af_heart",
        ),
        (
            vec![("LANG", "fr_FR.UTF-8"), ("LC_MESSAGES", "pt_BR.UTF-8")],
            vec!["--voices"],
            "pf_dora",
            "ff_siwis",
        ),
        (
            vec![
                ("LANG", "fr_FR.UTF-8"),
                ("LC_MESSAGES", "pt_BR.UTF-8"),
                ("LC_ALL", "zh_CN.UTF-8"),
            ],
            vec!["--voices"],
            "zf_xiaobei",
            "pf_dora",
        ),
        (
            vec![("LC_ALL", "fr_FR.UTF-8")],
            vec!["--voices", "--language", "en-GB"],
            "bf_emma",
            "af_heart",
        ),
        (
            vec![("LC_ALL", "C.UTF-8"), ("LANG", "fr_FR.UTF-8")],
            vec!["--voices"],
            "af_heart",
            "ff_siwis",
        ),
    ] {
        let result = invoke(&args, &locale);
        assert!(result.status.success());
        let list = String::from_utf8(result.stdout).unwrap();
        assert!(has_voice(&list, included, "kokoro"), "{list}");
        assert!(!has_voice(&list, excluded, "kokoro"), "{list}");
    }
    let all = invoke(
        &["--voices", "--all-languages"],
        &[("LC_ALL", "fr_FR.UTF-8")],
    );
    assert!(all.status.success());
    let all = String::from_utf8(all.stdout).unwrap();
    assert!(has_voice(&all, "af_heart", "kokoro"));
    assert!(has_voice(&all, "ff_siwis", "kokoro"));
    assert!(has_voice(&all, "zf_xiaobei", "kokoro"));
    assert!(has_voice(&all, "qwen-sohee", "qwen"));
    let english = run(&["--voices", "--language", "en"]);
    assert!(english.status.success());
    let english = String::from_utf8(english.stdout).unwrap();
    assert!(has_voice(&english, "af_heart", "kokoro"));
    assert!(has_voice(&english, "bf_emma", "kokoro"));
    for (name, language, gender) in [
        ("af_heart", "en-us", "female"),
        ("am_adam", "en-us", "male"),
        ("bf_emma", "en-gb", "female"),
        ("bm_george", "en-gb", "male"),
        ("ef_dora", "es", "female"),
        ("ff_siwis", "fr", "female"),
        ("hm_omega", "hi", "male"),
        ("if_sara", "it", "female"),
        ("pm_alex", "pt-br", "male"),
        ("zf_xiaobei", "zh", "female"),
    ] {
        let row = all
            .lines()
            .find(|line| line.split_whitespace().next() == Some(name))
            .unwrap();
        assert_eq!(
            row.split_whitespace().collect::<Vec<_>>(),
            [name, "kokoro", language, gender, "built-in"]
        );
        let info = run(&["--voice-info", name]);
        assert!(info.status.success());
        let info = String::from_utf8(info.stdout).unwrap();
        assert!(info.contains(&format!("Language: {language}\nGender: {gender}")));
    }
    // The column starts must agree even for long Piper names and short locale tags.
    let starts = |line: &str| -> Vec<usize> {
        line.char_indices()
            .filter_map(|(i, c)| {
                (!c.is_whitespace() && (i == 0 || line.as_bytes()[i - 1].is_ascii_whitespace()))
                    .then_some(i)
            })
            .collect()
    };
    let expected = starts(all.lines().next().unwrap());
    assert_eq!(expected.len(), 5);
    assert!(all.lines().all(|line| starts(line) == expected));
    assert!(!run(&["--voices", "--all-languages", "--language", "fr"])
        .status
        .success());
    assert!(!run(&["--all-languages", "Hello"]).status.success());
    assert!(!root.join("data").exists());
    for (name, engine, bundle) in [
        ("af_heart", "kokoro", "kokoro-multi-lang-v1_0"),
        (
            "en_US-lessac-medium",
            "piper",
            "vits-piper-en_US-lessac-medium",
        ),
        ("en_US-amy-low", "piper", "vits-piper-en_US-amy-low"),
        (
            "qwen-aiden",
            "qwen",
            "qwen-talker-1.7b-customvoice-BF16.gguf",
        ),
    ] {
        let list = run(&["--voices", "--engine", engine]);
        assert!(list.status.success());
        let list = String::from_utf8(list.stdout).unwrap();
        assert!(has_voice(&list, name, engine));
        assert!(!list.contains("alba"));
        assert!(run(&["--voice-info", name]).status.success());
        // Selecting a preset must pick its engine and model without explicit --engine.
        let missing = run(&["--voice", name, "Hello"]);
        assert!(!missing.status.success());
        let error = String::from_utf8_lossy(&missing.stderr);
        assert!(
            error.contains(bundle) && error.contains("enable auto_download"),
            "{error}"
        );
        let conflicting = run(&["--engine", "pocket", "--voice", name, "Hello"]);
        assert!(String::from_utf8_lossy(&conflicting.stderr).contains("conflicting"));
        assert!(!run(&["--remove-voice", name]).status.success());
    }
    assert!(!root.join("data").exists());
    for args in [
        vec!["--engine", "qwen", "--language", "invalid", "Hello"],
        vec!["--engine", "piper", "--instruct", "Speak warmly", "Hello"],
    ] {
        assert!(!run(&args).status.success());
    }

    let source = root.join("recording.wav");
    fs::write(&source, b"unused: model must be available before encoding").unwrap();
    for engine in ["kokoro", "piper"] {
        let unsupported = run(&[
            "--engine",
            engine,
            "--clone",
            source.to_str().unwrap(),
            "--name",
            "my-voice",
        ]);
        assert!(!unsupported.status.success());
        assert!(String::from_utf8_lossy(&unsupported.stderr).contains("only by Zonos"));
    }
    let failed = run(&["--clone", source.to_str().unwrap(), "--name", "my-voice"]);
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("enable auto_download"));
    let voices = root.join("data/tytts/voices");
    assert_eq!(fs::read_dir(&voices).unwrap().count(), 0);
    let failed = run(&[
        "--engine",
        "qwen",
        "--clone",
        source.to_str().unwrap(),
        "--name",
        "qwen-clone",
        "--transcript",
        "Hello.",
    ]);
    assert!(!failed.status.success());
    let error = String::from_utf8_lossy(&failed.stderr);
    assert!(
        error.contains("qwen-talker-1.7b-base-BF16.gguf") && error.contains("enable auto_download"),
        "{error}"
    );
    assert_eq!(fs::read_dir(&voices).unwrap().count(), 0);

    // A persisted voice from another invocation selects its engine and model.
    let saved = voices.join("my-voice");
    fs::create_dir(&saved).unwrap();
    fs::write(
        saved.join("manifest.json"),
        r#"{"version":1,"engine":"zonos","model":"previous-revision"}"#,
    )
    .unwrap();
    fs::write(saved.join("embedding.npy"), b"existing voice").unwrap();
    let list = run(&["--voices", "--engine", "zonos"]);
    assert!(list.status.success());
    let list = String::from_utf8(list.stdout).unwrap();
    assert!(has_voice(&list, "my-voice", "zonos"));
    let row = list
        .lines()
        .find(|line| line.starts_with("my-voice "))
        .unwrap();
    assert_eq!(
        row.split_whitespace().collect::<Vec<_>>(),
        ["my-voice", "zonos", "und", "und", "saved"]
    );
    let french = invoke(&["--voices"], &[("LANG", "fr_FR.UTF-8")]);
    assert!(french.status.success());
    let french = String::from_utf8(french.stdout).unwrap();
    assert!(has_voice(&french, "my-voice", "zonos"));
    assert!(!has_voice(&french, "savannah", "zonos"));
    assert!(!list.contains("alba"));
    assert!(run(&["--voice-info", "my-voice"]).status.success());
    let incompatible = run(&["--voice", "my-voice", "Hello"]);
    assert!(!incompatible.status.success());
    assert!(String::from_utf8_lossy(&incompatible.stderr).contains("previous-revision"));
    let collision = run(&["--clone", source.to_str().unwrap(), "--name", "my-voice"]);
    assert!(!collision.status.success());
    assert!(String::from_utf8_lossy(&collision.stderr).contains("already exists"));
    assert_eq!(
        fs::read(saved.join("embedding.npy")).unwrap(),
        b"existing voice"
    );
    assert!(!run(&["--remove-voice", "../my-voice"]).status.success());
    assert!(!run(&["--remove-voice", "alba"]).status.success());
    assert!(saved.is_dir());
    assert!(run(&["--remove-voice", "my-voice"]).status.success());
    assert!(!saved.exists());
}
