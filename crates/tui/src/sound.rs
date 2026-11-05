//! Alert sounds (`attention.sound`, off by default): 0.5.1's built-in MP3s, or the file named
//! in `attention.sounds.<name>`, played by the system's audio player so the binary carries no
//! audio stack.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sound {
    Done,
    SubagentDone,
    Question,
    Permission,
    Error,
}

impl Sound {
    pub fn name(self) -> &'static str {
        match self {
            Sound::Done => "done",
            Sound::SubagentDone => "subagent_done",
            Sound::Question => "question",
            Sound::Permission => "permission",
            Sound::Error => "error",
        }
    }

    fn builtin(self) -> (&'static str, &'static [u8]) {
        match self {
            Sound::Done => (
                "bip-bop-01.mp3",
                include_bytes!("../assets/sounds/bip-bop-01.mp3"),
            ),
            Sound::SubagentDone => ("yup-01.mp3", include_bytes!("../assets/sounds/yup-01.mp3")),
            Sound::Question => (
                "bip-bop-03.mp3",
                include_bytes!("../assets/sounds/bip-bop-03.mp3"),
            ),
            Sound::Permission => (
                "staplebops-06.mp3",
                include_bytes!("../assets/sounds/staplebops-06.mp3"),
            ),
            Sound::Error => (
                "nope-03.mp3",
                include_bytes!("../assets/sounds/nope-03.mp3"),
            ),
        }
    }
}

/// The player and its arguments for `file` at `volume` (0..1), from the first available one.
pub fn player(
    os: &str,
    available: impl Fn(&str) -> bool,
    file: &Path,
    volume: f64,
) -> Option<(String, Vec<String>)> {
    let file = file.display().to_string();
    let percent = ((volume.clamp(0.0, 1.0) * 100.0).round() as u32).to_string();
    let candidates: Vec<(&str, Vec<String>)> = match os {
        "macos" => vec![("afplay", vec!["-v".into(), format!("{volume:.2}"), file])],
        "windows" => {
            let file = file.replace('\'', "''");
            let script = format!(
                "Add-Type -AssemblyName PresentationCore; $p = New-Object System.Windows.Media.MediaPlayer; \
                 $p.Volume = {volume:.2}; $p.Open([Uri]'{file}'); $p.Play(); Start-Sleep -Milliseconds 1500"
            );
            vec![(
                "powershell",
                vec!["-NoProfile".into(), "-Command".into(), script],
            )]
        }
        _ => vec![
            (
                "paplay",
                vec![
                    format!("--volume={}", (volume.clamp(0.0, 1.0) * 65536.0) as u32),
                    file.clone(),
                ],
            ),
            (
                "pw-play",
                vec!["--volume".into(), format!("{volume:.2}"), file.clone()],
            ),
            (
                "ffplay",
                vec![
                    "-nodisp".into(),
                    "-autoexit".into(),
                    "-loglevel".into(),
                    "quiet".into(),
                    "-volume".into(),
                    percent.clone(),
                    file.clone(),
                ],
            ),
            (
                "mpv",
                vec![
                    "--no-video".into(),
                    "--really-quiet".into(),
                    format!("--volume={percent}"),
                    file,
                ],
            ),
        ],
    };
    candidates
        .into_iter()
        .find(|(program, _)| available(program))
        .map(|(program, args)| (program.to_string(), args))
}

/// The file to play: the override when it exists, else the built-in written once to `cache`.
fn file(sound: Sound, custom: Option<&Path>, cache: &Path) -> Option<PathBuf> {
    if let Some(custom) = custom.filter(|p| p.is_file()) {
        return Some(custom.to_path_buf());
    }
    let (name, bytes) = sound.builtin();
    let path = cache.join(name);
    if !path.is_file() {
        std::fs::create_dir_all(cache).ok()?;
        std::fs::write(&path, bytes).ok()?;
    }
    Some(path)
}

/// Plays without blocking; a missing player or file is silent.
pub fn play(sound: Sound, custom: Option<&Path>, cache: &Path, volume: f64) {
    let Some(path) = file(sound, custom, cache) else {
        return;
    };
    let available = |program: &str| anysola_core::which::which(program).is_some();
    let Some((program, args)) = player(std::env::consts::OS, available, &path, volume) else {
        return;
    };
    let child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Ok(mut child) = child {
        std::thread::spawn(move || child.wait());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_first_available_player() {
        let file = Path::new("/tmp/done.mp3");
        let (program, args) = player("linux", |p| p == "ffplay" || p == "mpv", file, 0.4).unwrap();
        assert_eq!(program, "ffplay");
        assert!(args.contains(&"40".to_string()));
        let (program, args) = player("macos", |_| true, file, 0.4).unwrap();
        assert_eq!((program.as_str(), args[1].as_str()), ("afplay", "0.40"));
        assert!(player("linux", |_| false, file, 0.4).is_none());
    }

    #[test]
    fn writes_the_builtin_once_and_prefers_an_override() {
        let dir = tempfile::tempdir().unwrap();
        let built = file(Sound::Permission, None, dir.path()).unwrap();
        assert_eq!(built.file_name().unwrap(), "staplebops-06.mp3");
        assert_eq!(std::fs::read(&built).unwrap().len(), 4971);
        let custom = dir.path().join("mine.wav");
        std::fs::write(&custom, b"x").unwrap();
        assert_eq!(file(Sound::Done, Some(&custom), dir.path()), Some(custom));
    }
}
