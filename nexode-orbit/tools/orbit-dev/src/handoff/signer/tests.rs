use super::*;
fn fixture() -> (tempfile::TempDir, Args) {
    let tmp = tempfile::tempdir().unwrap();
    let input = tmp.path().join("input");
    let mut bytes = vec![0; 32];
    bytes[..4].copy_from_slice(b"\xcf\xfa\xed\xfe");
    bytes[4..8].copy_from_slice(&0x0100000cu32.to_le_bytes());
    bytes[12] = 2;
    fs::write(&input, bytes).unwrap();
    let a = Args {
        sha256: digest(&input).unwrap(),
        input,
        output: tmp.path().join("signed"),
        target: "aarch64-apple-darwin".into(),
        identity: Some("AB".repeat(20)),
        identifier: "com.nexode.cli".into(),
        thumbprint: None,
        signtool: None,
        timestamp_url: "https://timestamp.digicert.com".into(),
        execute: false,
    };
    (tmp, a)
}
#[test]
fn default_plan_no_output_and_all_refusals_before_native() {
    let (tmp, a) = fixture();
    let out = sign(&a, "Darwin", |_, _| panic!("must not run")).unwrap();
    assert_eq!(out["mode"], "plan-only");
    assert!(!a.output.exists());
    assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 1);
    for mode in [
        "wrong-hash",
        "bad-hash",
        "identity",
        "architecture",
        "identifier",
        "host",
    ] {
        let mut b = a.clone();
        b.execute = true;
        match mode {
            "wrong-hash" => b.sha256 = "0".repeat(64),
            "bad-hash" => b.sha256 = "bad".into(),
            "identity" => b.identity = Some("-".into()),
            "architecture" => b.target = "x86_64-apple-darwin".into(),
            "identifier" => b.identifier = "bad\" or true".into(),
            _ => {}
        }
        assert!(
            sign(
                &b,
                if mode == "host" { "Linux" } else { "Darwin" },
                |_, _| panic!("must not run")
            )
            .is_err()
        );
        assert!(!b.output.exists());
        assert_eq!(digest(&b.input).unwrap(), a.sha256);
    }
}
#[test]
fn signing_copy_verification_failure_and_race_preserve_input() {
    let (tmp, mut a) = fixture();
    a.execute = true;
    let original = fs::read(&a.input).unwrap();
    let mut calls = 0;
    let out = sign(&a, "Darwin", |args, env| {
        assert!(!a.output.exists());
        assert!(!env.contains_key("AWS_SECRET_ACCESS_KEY"));
        assert!(!env.contains_key("CLOUDFLARE_API_TOKEN"));
        let artifact = Path::new(args.last().unwrap());
        assert_ne!(artifact, a.input);
        if calls == 0 {
            let mut bytes = fs::read(artifact)?;
            bytes.extend_from_slice(b"mock-signature");
            fs::write(artifact, bytes)?;
        } else {
            assert!(args.contains(&"--verify".into()));
            assert!(args[args.len() - 2].starts_with("=anchor apple generic"));
            assert!(fs::read(artifact)?.ends_with(b"mock-signature"));
        }
        calls += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(calls, 2);
    assert_eq!(out["mode"], "signed-and-native-verified");
    assert!(fs::read(&a.output).unwrap().ends_with(b"mock-signature"));
    assert_eq!(fs::read(&a.input).unwrap(), original);
    assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 2);
    for fail in [0, 1] {
        let (tmp, mut a) = fixture();
        a.execute = true;
        let mut calls = 0;
        assert!(
            sign(&a, "Darwin", |_, _| {
                let current = calls;
                calls += 1;
                ensure!(current != fail, "native failure");
                Ok(())
            })
            .is_err()
        );
        assert!(!a.output.exists());
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 1);
    }
    let (_tmp, mut a) = fixture();
    a.execute = true;
    assert!(
        sign(&a, "Darwin", |args, _| {
            if args.contains(&"--verify".into()) {
                fs::write(&a.output, "concurrent owner")?;
            }
            Ok(())
        })
        .is_err()
    );
    assert_eq!(fs::read_to_string(&a.output).unwrap(), "concurrent owner");
    assert_eq!(digest(&a.input).unwrap(), a.sha256);
}
#[test]
fn existing_output_and_windows_contract_and_headers() {
    let (_tmp, mut a) = fixture();
    fs::write(&a.output, "keep").unwrap();
    assert!(sign(&a, "Darwin", |_, _| unreachable!()).is_err());
    assert_eq!(fs::read_to_string(&a.output).unwrap(), "keep");
    a.output = a.input.clone();
    assert!(sign(&a, "Darwin", |_, _| unreachable!()).is_err());
    let (tmp, mut a) = fixture();
    let mut data = vec![0u8; 128];
    data[..2].copy_from_slice(b"MZ");
    data[60] = 64;
    data[64..68].copy_from_slice(b"PE\0\0");
    data[68..70].copy_from_slice(&0x8664u16.to_le_bytes());
    data[86] = 2;
    data[88..90].copy_from_slice(&0x20bu16.to_le_bytes());
    fs::write(&a.input, &data).unwrap();
    a.target = "x86_64-pc-windows-msvc".into();
    a.identity = None;
    a.thumbprint = Some("AB".repeat(20));
    a.signtool = Some(tmp.path().join("signtool.exe"));
    a.sha256 = digest(&a.input).unwrap();
    a.execute = true;
    let mut calls = vec![];
    sign(&a, "Windows", |args, _| {
        calls.push(args.to_vec());
        Ok(())
    })
    .unwrap();
    assert_eq!(&calls[0][1..5], ["sign", "/s", "My", "/sha1"]);
    assert_eq!(&calls[1][1..5], ["verify", "/pa", "/all", "/v"]);
    assert!(!calls[0].contains(&"/p".into()));
    assert!(!calls[0].contains(&"/f".into()));
    for mode in 0..3 {
        let mut b = a.clone();
        match mode {
            0 => b.thumbprint = Some("bad".into()),
            1 => b.signtool = Some("relative.exe".into()),
            _ => b.timestamp_url = "https://user:password@example.com".into(),
        };
        assert!(commands(&b, "artifact").is_err());
    }
    for bytes in [vec![], b"MZ".to_vec(), b"\x7fELF".to_vec()] {
        fs::write(&a.input, bytes).unwrap();
        assert!(header(&a.input, &a.target).is_err());
    }
}
