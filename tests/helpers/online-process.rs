use std::{env, fs, io::Write, path::PathBuf, process::Command, thread, time::Duration};

fn main() {
    let mut args = env::args_os().skip(1);
    match args.next().as_deref().and_then(|value| value.to_str()) {
        None => {
            let home = env::var_os("HOME").expect("isolated home");
            assert_eq!(env::var_os("APPDATA").as_ref(), Some(&home));
            assert_eq!(env::var_os("USERPROFILE").as_ref(), Some(&home));
            assert_eq!(env::var_os("TEMP").as_ref(), Some(&home));
            assert!(env::var_os("PATH").is_none(), "inherited PATH");
            assert_eq!(env::current_dir().unwrap(), PathBuf::from(home));
            println!("isolated");
        }
        Some("tree") => {
            let directory = PathBuf::from(args.next().unwrap());
            let mut child = Command::new(env::current_exe().unwrap())
                .arg("child")
                .arg(&directory)
                .spawn()
                .unwrap();
            while !directory.join("release").exists() {
                thread::sleep(Duration::from_millis(10));
            }
            println!("{}", "x".repeat(2048));
            std::io::stdout().flush().unwrap();
            child.wait().unwrap();
        }
        Some("child") => {
            let directory = PathBuf::from(args.next().unwrap());
            fs::write(directory.join("child.tmp"), std::process::id().to_string()).unwrap();
            fs::rename(directory.join("child.tmp"), directory.join("child.pid")).unwrap();
            thread::sleep(Duration::from_secs(120));
        }
        mode => panic!("Unexpected fixture mode: {mode:?}"),
    }
}
