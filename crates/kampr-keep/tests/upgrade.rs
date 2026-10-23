mod common;

use common::{EXE, Live, proc_alive, root, sh, until};
use kampr_keep::{How, Image, Launch, SessionDir, stale, upgrade_stale, upgrade_while};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const SOON: Duration = Duration::from_secs(10);

// A copy open for writing is inherited by any fork a parallel test makes meanwhile, and exec'ing
// that file then fails with ETXTBSY.
static FORKS: Mutex<()> = Mutex::new(());

fn install(from: &Path, at: &Path) {
    let _forks = FORKS.lock().unwrap_or_else(|e| e.into_inner());
    let staged = at.with_extension("new");
    std::fs::copy(from, &staged).expect("copy");
    std::fs::rename(&staged, at).expect("rename");
}

fn installed_keeper(root: &Path) -> PathBuf {
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).expect("bin");
    let exe = bin.join("kampr-keep");
    install(Path::new(EXE), &exe);
    exe
}

fn spawn_with(root: &Path, exe: &Path) -> SessionDir {
    let _forks = FORKS.lock().unwrap_or_else(|e| e.into_inner());
    kampr_keep::spawn(root, sh("exec cat"), Launch::Detached, exe).expect("spawn")
}

#[test]
fn a_keeper_whose_binary_was_replaced_is_re_execd_into_the_new_one_by_the_upgrade_loop() {
    let root = root();
    let exe = installed_keeper(root.path());
    let session = spawn_with(root.path(), &exe);
    let live = Live::adopt(session, Launch::Detached);
    let (keeper, child) = (live.keeper_pid(), live.child());
    assert!(
        !stale(&live.session),
        "a keeper running the installed binary is not stale"
    );
    install(Path::new(EXE), &exe);
    assert!(
        stale(&live.session),
        "the binary at its path is no longer the one it runs"
    );

    let passes = Arc::new(AtomicBool::new(true));
    let running = passes.clone();
    let looping = upgrade_while(root.path().to_path_buf(), Duration::from_millis(50), move |_| {
        running.load(Ordering::Relaxed)
    });
    assert!(
        until(SOON, || live
            .session
            .keeper()
            .is_some_and(|k| k.how == How::ReExec)),
        "never re-exec'd: {:?}",
        live.session.keeper()
    );
    let written = || {
        std::fs::metadata(live.session.path().join("keeper.json"))
            .and_then(|m| m.modified())
            .expect("keeper.json")
    };
    assert!(
        until(SOON, || !stale(&live.session)),
        "still stale after the re-exec"
    );
    std::thread::sleep(Duration::from_millis(300));
    let settled = written();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(written(), settled, "an upgraded keeper was signalled again");
    passes.store(false, Ordering::Relaxed);
    looping.join().expect("upgrade loop");

    let info = live.session.keeper().expect("keeper.json");
    assert_eq!((info.keeper, info.child), (keeper, child));
    assert!(proc_alive(child));
    assert_eq!(upgrade_stale(root.path()).expect("pass"), Vec::<String>::new());
    kampr_keep::borrow(&live.session).expect("the upgraded keeper answers");
}

#[test]
fn a_binary_deleted_with_nothing_in_its_place_is_left_alone() {
    let root = root();
    let exe = installed_keeper(root.path());
    let live = Live::adopt(spawn_with(root.path(), &exe), Launch::Detached);
    std::fs::remove_file(&exe).expect("remove");
    assert!(!stale(&live.session), "nothing to re-exec into");
    assert_eq!(upgrade_stale(root.path()).expect("pass"), Vec::<String>::new());
}

#[test]
fn a_pid_that_is_no_longer_its_keeper_is_never_signalled() {
    let root = root();
    let exe = installed_keeper(root.path());
    let sleeper_exe = root.path().join("bin").join("sleep");
    install(Path::new("/bin/sleep"), &sleeper_exe);
    let mut sleeper = std::process::Command::new(&sleeper_exe)
        .arg("30")
        .spawn()
        .expect("sleep");
    let live = Live::adopt(spawn_with(root.path(), &exe), Launch::Detached);
    let path = live.session.path().join("keeper.json");
    let real = live.session.keeper().expect("keeper.json");
    let mut info = real.clone();
    info.keeper = sleeper.id();
    info.exe = sleeper_exe.to_string_lossy().into_owned();
    std::fs::write(&path, serde_json::to_vec(&info).expect("json")).expect("write");
    install(Path::new("/bin/sleep"), &sleeper_exe);

    assert!(
        !stale(&live.session),
        "a pid running something other than `keep <dir>`"
    );
    assert_eq!(upgrade_stale(root.path()).expect("pass"), Vec::<String>::new());
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        sleeper.try_wait().expect("try_wait").is_none(),
        "the sleeper was signalled"
    );
    let _ = sleeper.kill();
    std::fs::write(&path, serde_json::to_vec(&real).expect("json")).expect("restore");
}

#[test]
fn a_keeper_in_a_nested_root_is_upgraded_by_a_pass_over_the_top_one() {
    let root = root();
    let exe = installed_keeper(root.path());
    kampr_keep::make_private_dir(root.path()).expect("root");
    let nested = root.path().join("fleet").join("default");
    kampr_keep::make_private_dir(&nested).expect("nested root");
    let live = Live::adopt(spawn_with(&nested, &exe), Launch::Detached);
    install(Path::new(EXE), &exe);
    assert_eq!(
        upgrade_stale(root.path()).expect("pass"),
        vec![live.session.id().to_string()]
    );
    assert!(until(SOON, || live
        .session
        .keeper()
        .is_some_and(|k| k.how == How::ReExec)));
}

/// A Mac has no `/proc/<pid>/exe`, so a keeper records the file it started from and is judged by
/// that; a keeper from before the record, on Linux, is still judged by `/proc`.
#[test]
fn a_keeper_is_judged_by_the_binary_it_recorded_and_without_a_record_by_proc() {
    let root = root();
    let exe = installed_keeper(root.path());
    let live = Live::adopt(spawn_with(root.path(), &exe), Launch::Detached);
    let path = live.session.path().join("keeper.json");
    let real = live.session.keeper().expect("keeper.json");
    let installed = std::fs::metadata(&exe).expect("the installed keeper");
    assert_eq!(
        real.image,
        Some(Image::of(&installed)),
        "a keeper records the binary it runs"
    );
    let rewrite = |info: &kampr_keep::KeeperInfo| {
        std::fs::write(&path, serde_json::to_vec(info).expect("json")).expect("write")
    };

    let mut elsewhere = real.clone();
    elsewhere.image = Some(Image { dev: 0, ino: 1 });
    rewrite(&elsewhere);
    assert!(stale(&live.session), "the record, not /proc, says what it runs");

    let mut unrecorded = real.clone();
    unrecorded.image = None;
    rewrite(&unrecorded);
    assert!(!stale(&live.session), "/proc says it runs the installed binary");
    install(Path::new(EXE), &exe);
    assert!(stale(&live.session), "/proc says the binary was replaced");
    rewrite(&real);
}
