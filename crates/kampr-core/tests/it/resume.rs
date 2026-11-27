use kampr_core::resume::{Action, Command, Freshness, Report, Token, Verdict, read};
use kampr_testkit::resume::sign;

const NOW: i64 = 1_795_543_200;
const LIVE: Freshness = Freshness::Live { now: NOW };

fn token() -> Token {
    Token::parse("dummy-token-replace-me").expect("a token")
}

fn b64(text: &str) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(text)
}

fn verdict(body: &str) -> Verdict {
    read(sign(body, "dummy-token-replace-me").as_bytes(), &token(), LIVE)
}

fn accepted(pid: u32, time: i64, action: Action) -> Verdict {
    Verdict::Accepted(Report { pid, time, action })
}

fn set(argv: &[&str], cwd: Option<&str>) -> Action {
    Action::Set(Command {
        argv: argv.iter().map(|a| a.to_string()).collect(),
        cwd: cwd.map(str::to_string),
    })
}

#[test]
fn the_specs_worked_example_verifies() {
    let published = "pid=74877:time=1795543200:clear=1:mac=c30603369f98fef581a4098be889175b0c74f0641e49f8b25f4326cb38f97c93";
    assert_eq!(
        read(published.as_bytes(), &token(), LIVE),
        accepted(74877, NOW, Action::Clear)
    );
    let other = Token::parse("another-token-0123456789").unwrap();
    assert_eq!(read(published.as_bytes(), &other, LIVE), Verdict::Ignored);
}

#[test]
fn an_argv_and_a_cwd_come_back_as_the_program_wrote_them() {
    let argv = ["nvim", "-S", "/home/u/session.vim", "", "a b;$(reboot)"]
        .map(b64)
        .join(",");
    let cwd = b64("/home/u/project");
    assert_eq!(
        verdict(&format!("pid=7:time={NOW}:argv={argv}:cwd={cwd}")),
        accepted(
            7,
            NOW,
            set(
                &["nvim", "-S", "/home/u/session.vim", "", "a b;$(reboot)"],
                Some("/home/u/project")
            )
        )
    );
    assert_eq!(
        verdict(&format!(
            "cwd={cwd}:time={NOW}:argv={}:pid=7",
            b64("/usr/bin/nvim")
        )),
        accepted(7, NOW, set(&["/usr/bin/nvim"], Some("/home/u/project"))),
        "keys in any order but mac"
    );
    assert_eq!(
        verdict(&format!("pid=7:time={NOW}:argv=bnZpbQ:future=whatever")),
        accepted(7, NOW, set(&["nvim"], None)),
        "unpadded base64 and an unknown key"
    );
}

#[test]
fn a_report_without_a_good_mac_does_nothing() {
    let body = format!("pid=74877:time={NOW}:argv=bnZpbQ==");
    let good = sign(&body, "dummy-token-replace-me");
    let mac = good.rsplit_once(":mac=").unwrap().1;
    let cases = [
        ("a wrong mac", format!("{body}:mac={}", "0".repeat(64))),
        ("no mac", body.clone()),
        ("a mac that is not last", format!("mac={mac}:{body}")),
        ("a short mac", format!("{body}:mac={}", &mac[..63])),
        ("an upper-case mac", format!("{body}:mac={}", mac.to_uppercase())),
        ("a mac over a changed body", good.replace("74877", "74878")),
        ("the wrong token", sign(&body, "some-other-token-0123456789")),
        (
            "a report with a tail after the mac",
            format!("{good}:argv=bnZpbQ=="),
        ),
        ("nothing", String::new()),
        ("the reserved query body", "?".into()),
    ];
    for (why, report) in cases {
        assert_eq!(read(report.as_bytes(), &token(), LIVE), Verdict::Ignored, "{why}");
    }
    assert_eq!(read(&[0xff, 0xfe], &token(), LIVE), Verdict::Ignored);
}

#[test]
fn an_authentic_report_that_is_not_fresh_or_not_whole_does_nothing() {
    let argv = b64("nvim");
    for (why, body) in [
        (
            "a time too far back",
            format!("pid=7:time={}:argv={argv}", NOW - 61),
        ),
        (
            "a time too far ahead",
            format!("pid=7:time={}:argv={argv}", NOW + 61),
        ),
        ("no time", format!("pid=7:argv={argv}")),
        (
            "a time with a leading zero",
            format!("pid=7:time=0{NOW}:argv={argv}"),
        ),
        ("no pid", format!("time={NOW}:argv={argv}")),
        ("a pid of zero", format!("pid=0:time={NOW}:argv={argv}")),
        ("neither argv nor clear", format!("pid=7:time={NOW}")),
    ] {
        assert_eq!(verdict(&body), Verdict::Ignored, "{why}");
    }
    for edge in [NOW - 60, NOW + 60] {
        assert_eq!(
            verdict(&format!("pid=7:time={edge}:argv={argv}")),
            accepted(7, edge, set(&["nvim"], None)),
            "{edge}"
        );
    }
}

#[test]
fn a_replayed_report_is_read_whatever_its_age() {
    let body = sign(
        &format!("pid=7:time=1000000000:argv={}", b64("nvim")),
        "dummy-token-replace-me",
    );
    assert_eq!(read(body.as_bytes(), &token(), LIVE), Verdict::Ignored);
    assert_eq!(
        read(body.as_bytes(), &token(), Freshness::Replayed),
        accepted(7, 1_000_000_000, set(&["nvim"], None))
    );
}

#[test]
fn an_authentic_fresh_report_that_breaks_a_rule_rejects_its_owner() {
    let long: Vec<String> = (0..129).map(|n| b64(&format!("a{n}"))).collect();
    let cases = [
        (
            "a relative first element",
            format!("pid=9:time={NOW}:argv={}", b64("./run.sh")),
        ),
        (
            "a relative path with a slash",
            format!("pid=9:time={NOW}:argv={}", b64("bin/run")),
        ),
        (
            "an empty first element",
            format!("pid=9:time={NOW}:argv={},{}", b64(""), b64("x")),
        ),
        ("argv with nothing", format!("pid=9:time={NOW}:argv=")),
        (
            "argv and clear",
            format!("pid=9:time={NOW}:argv=bnZpbQ==:clear=1"),
        ),
        ("clear that is not 1", format!("pid=9:time={NOW}:clear=2")),
        (
            "a relative cwd",
            format!("pid=9:time={NOW}:argv=bnZpbQ==:cwd={}", b64("project")),
        ),
        (
            "a need nobody implements",
            format!("pid=9:time={NOW}:argv=bnZpbQ==:need=later"),
        ),
        (
            "a need among those implemented and one not",
            format!("pid=9:time={NOW}:argv=bnZpbQ==:need=cwd,later"),
        ),
        (
            "a repeated key",
            format!("pid=9:time={NOW}:argv=bnZpbQ==:argv=bnZpbQ=="),
        ),
        ("bad base64", format!("pid=9:time={NOW}:argv=!!!!")),
        ("wrong padding", format!("pid=9:time={NOW}:argv=bnZpbQ=")),
        (
            "bytes that are not UTF-8",
            format!("pid=9:time={NOW}:argv={}", base64_bytes(&[0xff, 0xfe])),
        ),
        (
            "a control character",
            format!("pid=9:time={NOW}:argv={}", b64("a\nb")),
        ),
        (
            "a C1 control",
            format!("pid=9:time={NOW}:argv={}", b64("a\u{85}b")),
        ),
        (
            "a control character in cwd",
            format!("pid=9:time={NOW}:argv=bnZpbQ==:cwd={}", b64("/a\tb")),
        ),
        (
            "too many elements",
            format!("pid=9:time={NOW}:argv={}", long.join(",")),
        ),
        (
            "a pair that is not key=value",
            format!("pid=9:time={NOW}:argv=bnZpbQ==:junk"),
        ),
        (
            "a character outside the value set",
            format!("pid=9:time={NOW}:argv=bnZp bQ=="),
        ),
    ];
    for (why, body) in cases {
        assert_eq!(verdict(&body), Verdict::Rejected { pid: 9 }, "{why}");
    }
}

fn base64_bytes(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[test]
fn needs_the_terminal_implements_pass() {
    assert_eq!(
        verdict(&format!("pid=9:time={NOW}:argv=bnZpbQ==:need=cwd,argv")),
        accepted(9, NOW, set(&["nvim"], None))
    );
}

#[test]
fn tokens_are_random_and_in_the_alphabet_the_spec_gives() {
    let a = Token::generate().unwrap();
    let b = Token::generate().unwrap();
    assert_ne!(a, b);
    assert_eq!(a.as_str().len(), 22);
    assert!(Token::parse(a.as_str()).is_some());
    for bad in [
        "",
        "short",
        &"a".repeat(65),
        "has space and is long enough",
        "semi;colon-and-long-enough",
    ] {
        assert!(Token::parse(bad).is_none(), "{bad:?}");
    }
    assert!(!format!("{a:?}").contains(a.as_str()));
}
