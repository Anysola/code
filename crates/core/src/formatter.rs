//! Formatters run after `edit` and `write` (config `formatter`, off unless set): the first
//! formatter whose extensions contain the file's extension and that exits 0 wins. Their output is
//! never shown; failures are only logged.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;

use serde_json::Value;

use crate::which::which_in;

const JS: &[&str] = &[".js", ".jsx", ".mjs", ".cjs", ".ts", ".tsx", ".mts", ".cts"];
const WEB: &[&str] = &[
    ".js", ".jsx", ".mjs", ".cjs", ".ts", ".tsx", ".mts", ".cts", ".html", ".htm", ".css", ".scss",
    ".sass", ".less", ".vue", ".svelte", ".json", ".jsonc", ".yaml", ".yml", ".toml", ".xml",
    ".md", ".mdx", ".graphql", ".gql",
];
const RUBY: &[&str] = &[".rb", ".rake", ".gemspec", ".ru"];

/// How a formatter finds its command.
#[derive(Debug, Clone, PartialEq)]
enum Detect {
    /// An executable on PATH (or in the data dir's `bin`), run with these arguments.
    Executable(&'static str, &'static [&'static str]),
    /// A package's bin under `node_modules/.bin`, when a `package.json` up the tree depends on it.
    Npm {
        package: &'static str,
        bin: &'static str,
        args: &'static [&'static str],
    },
    Biome,
    /// The executable, only when one of these files is found up the tree.
    WithConfig(&'static str, &'static str, &'static [&'static str]),
    Ruff,
    Air,
    Uv,
    Pint,
    /// A configured command.
    Command(Vec<String>),
    Disabled,
}

#[derive(Debug, Clone, PartialEq)]
struct Formatter {
    name: String,
    extensions: Vec<String>,
    environment: Vec<(String, String)>,
    detect: Detect,
}

fn builtin(name: &str, extensions: &[&str], detect: Detect) -> Formatter {
    let npm = matches!(detect, Detect::Npm { .. } | Detect::Biome);
    Formatter {
        name: name.into(),
        extensions: extensions.iter().map(|e| e.to_string()).collect(),
        environment: if npm {
            vec![("BUN_BE_BUN".into(), "1".into())]
        } else {
            Vec::new()
        },
        detect,
    }
}

fn builtins() -> Vec<Formatter> {
    use Detect::*;
    vec![
        builtin("gofmt", &[".go"], Executable("gofmt", &["-w"])),
        builtin(
            "mix",
            &[".ex", ".exs", ".eex", ".heex", ".leex", ".neex", ".sface"],
            Executable("mix", &["format"]),
        ),
        builtin(
            "oxfmt",
            JS,
            Npm {
                package: "oxfmt",
                bin: "oxfmt",
                args: &[],
            },
        ),
        builtin(
            "prettier",
            WEB,
            Npm {
                package: "prettier",
                bin: "prettier",
                args: &["--write"],
            },
        ),
        builtin("biome", WEB, Biome),
        builtin("zig", &[".zig", ".zon"], Executable("zig", &["fmt"])),
        builtin(
            "clang-format",
            &[
                ".c", ".cc", ".cpp", ".cxx", ".c++", ".h", ".hh", ".hpp", ".hxx", ".h++", ".ino",
                ".C", ".H",
            ],
            WithConfig(".clang-format", "clang-format", &["-i"]),
        ),
        builtin("ktlint", &[".kt", ".kts"], Executable("ktlint", &["-F"])),
        builtin("ruff", &[".py", ".pyi"], Ruff),
        builtin("air", &[".R"], Air),
        builtin("uv", &[".py", ".pyi"], Uv),
        builtin("rubocop", RUBY, Executable("rubocop", &["--autocorrect"])),
        builtin("standardrb", RUBY, Executable("standardrb", &["--fix"])),
        builtin(
            "htmlbeautifier",
            &[".erb", ".html.erb"],
            Executable("htmlbeautifier", &[]),
        ),
        builtin("dart", &[".dart"], Executable("dart", &["format"])),
        builtin(
            "ocamlformat",
            &[".ml", ".mli"],
            WithConfig(".ocamlformat", "ocamlformat", &["-i"]),
        ),
        builtin(
            "terraform",
            &[".tf", ".tfvars"],
            Executable("terraform", &["fmt"]),
        ),
        builtin(
            "latexindent",
            &[".tex"],
            Executable("latexindent", &["-w", "-s"]),
        ),
        builtin("gleam", &[".gleam"], Executable("gleam", &["format"])),
        builtin("shfmt", &[".sh", ".bash"], Executable("shfmt", &["-w"])),
        builtin("nixfmt", &[".nix"], Executable("nixfmt", &[])),
        builtin("rustfmt", &[".rs"], Executable("rustfmt", &[])),
        builtin("pint", &[".php"], Pint),
        builtin("ormolu", &[".hs"], Executable("ormolu", &["-i"])),
        builtin(
            "cljfmt",
            &[".clj", ".cljs", ".cljc", ".edn"],
            Executable("cljfmt", &["fix", "--quiet"]),
        ),
        builtin("dfmt", &[".d"], Executable("dfmt", &["-i"])),
    ]
}

fn strings(value: Option<&Value>) -> Option<Vec<String>> {
    value.and_then(Value::as_array).map(|items| {
        items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    })
}

/// The configured formatters, or `None` when config `formatter` is absent or `false`.
pub struct Formatters {
    list: Vec<Formatter>,
    /// Data-dir `bin`, searched after PATH.
    bin: PathBuf,
    /// Detected commands per (formatter, location directory); misses are retried.
    commands: Mutex<HashMap<(String, PathBuf), Vec<String>>>,
}

impl Formatters {
    pub fn from_config(config: &crate::config::Config, bin: PathBuf) -> Option<Formatters> {
        let configured = config.get("formatter")?;
        let mut list = builtins();
        match configured {
            Value::Bool(true) => {}
            Value::Object(entries) => {
                for (name, entry) in entries {
                    if entry.get("disabled").and_then(Value::as_bool) == Some(true) {
                        list.retain(|f| &f.name != name);
                        continue;
                    }
                    let builtin = list.iter().find(|f| &f.name == name).cloned();
                    let command = strings(entry.get("command"));
                    let mut environment = builtin
                        .as_ref()
                        .map(|b| b.environment.clone())
                        .unwrap_or_default();
                    if let Some(Value::Object(extra)) = entry.get("environment") {
                        for (key, value) in extra {
                            environment.retain(|(k, _)| k != key);
                            environment
                                .push((key.clone(), value.as_str().unwrap_or_default().into()));
                        }
                    }
                    let formatter = Formatter {
                        name: name.clone(),
                        extensions: strings(entry.get("extensions"))
                            .or_else(|| builtin.as_ref().map(|b| b.extensions.clone()))
                            .unwrap_or_default(),
                        environment,
                        detect: match (command, &builtin) {
                            (Some(command), _) => Detect::Command(command),
                            (None, Some(builtin)) => builtin.detect.clone(),
                            (None, None) => Detect::Disabled,
                        },
                    };
                    match list.iter_mut().find(|f| f.name == *name) {
                        Some(slot) => *slot = formatter,
                        None => list.push(formatter),
                    }
                }
            }
            _ => return None,
        }
        Some(Formatters {
            list,
            bin,
            commands: Mutex::default(),
        })
    }

    /// Formats `file` (absolute) from the location `directory` within `worktree`; true when a
    /// formatter succeeded.
    pub async fn format(&self, file: &Path, directory: &Path, worktree: &Path) -> bool {
        let extension = extension(file);
        let place = Place {
            directory,
            worktree,
            bin: &self.bin,
        };
        for formatter in self
            .list
            .iter()
            .filter(|f| f.extensions.contains(&extension))
        {
            let key = (formatter.name.clone(), directory.to_path_buf());
            let cached = self
                .commands
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&key)
                .cloned();
            let command = match cached {
                Some(command) => command,
                None => match detect(&formatter.detect, &place).await {
                    Some(command) => {
                        self.commands
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .insert(key, command.clone());
                        command
                    }
                    None => continue,
                },
            };
            let file = file.to_string_lossy();
            let argv: Vec<String> = command
                .iter()
                .map(|a| a.replacen("$FILE", &file, 1))
                .collect();
            crate::log::info(
                "formatting file",
                &[("file", &file), ("command", &argv.join(" "))],
            );
            let status = tokio::process::Command::new(&argv[0])
                .args(&argv[1..])
                .current_dir(directory)
                .envs(formatter.environment.iter().cloned())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await;
            match status {
                Ok(status) if status.success() => return true,
                Ok(status) => crate::log::error(
                    "formatter exited unsuccessfully",
                    &[
                        ("file", &file),
                        ("exitCode", &format!("{:?}", status.code())),
                    ],
                ),
                Err(error) => crate::log::error(
                    "failed to format file",
                    &[("file", &file), ("error", &error.to_string())],
                ),
            }
        }
        false
    }
}

/// `path.extname`: the last `.` suffix of the file name, unless the name starts with it.
fn extension(file: &Path) -> String {
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    match name.rfind('.') {
        Some(index) if index > 0 => name[index..].to_string(),
        _ => String::new(),
    }
}

struct Place<'a> {
    directory: &'a Path,
    worktree: &'a Path,
    bin: &'a Path,
}

impl Place<'_> {
    /// `target` in the location directory and each parent up to the worktree root.
    fn find_up(&self, target: &str) -> Vec<PathBuf> {
        let mut found = Vec::new();
        for dir in self.directory.ancestors() {
            let candidate = dir.join(target);
            if candidate.exists() {
                found.push(candidate);
            }
            if dir == self.worktree {
                break;
            }
        }
        found
    }

    fn executable(&self, name: &str) -> Option<String> {
        which_in(name, Some(self.bin)).map(|p| p.to_string_lossy().into_owned())
    }

    /// The package's bin in the nearest `node_modules/.bin` up the tree.
    fn npm_bin(&self, bin: &str) -> Option<String> {
        self.find_up("node_modules/.bin")
            .into_iter()
            .map(|dir| dir.join(bin))
            .find(|path| path.exists())
            .map(|p| p.to_string_lossy().into_owned())
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap_or_default()
    }

    fn json(path: &Path) -> Value {
        serde_json::from_str(&Self::read(path)).unwrap_or(Value::Null)
    }
}

fn has_key(json: &Value, field: &str, key: &str) -> bool {
    json.get(field)
        .and_then(Value::as_object)
        .is_some_and(|m| m.contains_key(key))
}

fn with_file(command: String, args: &[&str]) -> Vec<String> {
    let mut argv = vec![command];
    argv.extend(args.iter().map(|a| a.to_string()));
    argv.push("$FILE".into());
    argv
}

async fn output(argv: &[&str], directory: &Path) -> Option<std::process::Output> {
    tokio::process::Command::new(argv[0])
        .args(&argv[1..])
        .current_dir(directory)
        .stdin(Stdio::null())
        .output()
        .await
        .ok()
}

async fn detect(detect: &Detect, place: &Place<'_>) -> Option<Vec<String>> {
    match detect {
        Detect::Executable(name, args) => Some(with_file(place.executable(name)?, args)),
        Detect::Npm { package, bin, args } => {
            let depends = place.find_up("package.json").iter().any(|file| {
                let json = Place::json(file);
                has_key(&json, "dependencies", package)
                    || has_key(&json, "devDependencies", package)
            });
            depends
                .then(|| place.npm_bin(bin))
                .flatten()
                .map(|bin| with_file(bin, args))
        }
        Detect::Biome => {
            let configured = ["biome.json", "biome.jsonc"]
                .iter()
                .any(|name| !place.find_up(name).is_empty());
            configured
                .then(|| place.npm_bin("biome"))
                .flatten()
                .map(|bin| with_file(bin, &["format", "--write"]))
        }
        Detect::WithConfig(config, name, args) => {
            if place.find_up(config).is_empty() {
                return None;
            }
            Some(with_file(place.executable(name)?, args))
        }
        Detect::Ruff => {
            let bin = place.executable("ruff")?;
            let command = || Some(with_file(bin.clone(), &["format"]));
            for config in ["pyproject.toml", "ruff.toml", ".ruff.toml"] {
                let Some(found) = place.find_up(config).into_iter().next() else {
                    continue;
                };
                if config != "pyproject.toml" || Place::read(&found).contains("[tool.ruff]") {
                    return command();
                }
            }
            for dependency in ["requirements.txt", "pyproject.toml", "Pipfile"] {
                if let Some(found) = place.find_up(dependency).into_iter().next()
                    && Place::read(&found).contains("ruff")
                {
                    return command();
                }
            }
            None
        }
        Detect::Air => {
            let bin = place.executable("air")?;
            let out = output(&[&bin, "--help"], place.directory).await?;
            let stdout = String::from_utf8_lossy(&out.stdout);
            let first = stdout.split('\n').next().unwrap_or_default();
            (out.status.success() && first.contains("R language") && first.contains("formatter"))
                .then(|| with_file(bin.clone(), &["format"]))
        }
        Detect::Uv => {
            let bin = place.executable("uv")?;
            let out = output(&[&bin, "format", "--help"], place.directory).await?;
            out.status
                .success()
                .then(|| with_file(bin.clone(), &["format", "--"]))
        }
        Detect::Pint => place
            .find_up("composer.json")
            .iter()
            .map(|file| Place::json(file))
            .any(|json| {
                has_key(&json, "require", "laravel/pint")
                    || has_key(&json, "require-dev", "laravel/pint")
            })
            .then(|| with_file("./vendor/bin/pint".into(), &[])),
        Detect::Command(command) => Some(command.clone()),
        Detect::Disabled => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn formatters(config: &str) -> Option<Formatters> {
        let config = Config::from_documents(&[serde_json::from_str(config).unwrap()]);
        Formatters::from_config(&config, PathBuf::from("/nonexistent"))
    }

    #[test]
    fn extensions_follow_extname() {
        assert_eq!(extension(Path::new("/a/b.html.erb")), ".erb");
        assert_eq!(extension(Path::new("/a/.bashrc")), "");
        assert_eq!(extension(Path::new("/a/Makefile")), "");
        assert_eq!(extension(Path::new("/a/x.R")), ".R");
    }

    #[test]
    fn configuration() {
        assert!(formatters("{}").is_none());
        assert!(formatters(r#"{"formatter": false}"#).is_none());
        let all = formatters(r#"{"formatter": true}"#).unwrap();
        let names: Vec<_> = all.list.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names.len(), 26);
        assert_eq!(&names[..4], ["gofmt", "mix", "oxfmt", "prettier"]);
        assert_eq!(names.last(), Some(&"dfmt"));
        let custom = formatters(
            r#"{"formatter": {"gofmt": {"disabled": true}, "prettier": {"environment": {"A": "1"}},
                "mine": {"command": ["fmt", "$FILE"], "extensions": [".x"]}, "bare": {}}}"#,
        )
        .unwrap();
        assert!(!custom.list.iter().any(|f| f.name == "gofmt"));
        let prettier = custom.list.iter().find(|f| f.name == "prettier").unwrap();
        assert_eq!(
            prettier.environment,
            [("BUN_BE_BUN".into(), "1".into()), ("A".into(), "1".into())]
        );
        let mine = custom.list.iter().find(|f| f.name == "mine").unwrap();
        assert_eq!(
            mine.detect,
            Detect::Command(vec!["fmt".into(), "$FILE".into()])
        );
        let bare = custom.list.iter().find(|f| f.name == "bare").unwrap();
        assert_eq!(
            (bare.extensions.len(), &bare.detect),
            (0, &Detect::Disabled)
        );
    }

    /// Later documents: a boolean resets earlier objects, objects merge field by field.
    #[test]
    fn documents_merge() {
        let config = Config::from_documents(&[
            serde_json::json!({"formatter": {"mine": {"command": ["a"], "environment": {"X": "1"}}}}),
            serde_json::json!({"formatter": {"mine": {"extensions": [".x"], "environment": {"Y": "2"}}}}),
        ]);
        assert_eq!(
            config.get("formatter").unwrap(),
            &serde_json::json!({"mine": {"command": ["a"], "environment": {"X": "1", "Y": "2"}, "extensions": [".x"]}})
        );
        let config = Config::from_documents(&[
            serde_json::json!({"formatter": {"mine": {"command": ["a"]}}}),
            serde_json::json!({"formatter": true}),
        ]);
        assert_eq!(config.get("formatter"), Some(&Value::Bool(true)));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn first_success_wins_with_the_file_substituted() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.x");
        std::fs::write(&file, "x").unwrap();
        let list = formatters(
            r#"{"formatter": {"fails": {"command": ["false", "$FILE"], "extensions": [".x"]},
                "upper": {"command": ["sh", "-c", "tr a-z A-Z < \"$0\" > \"$0.t\" && mv \"$0.t\" \"$0\"; pwd > where", "$FILE"], "extensions": [".x"]},
                "never": {"command": ["sh", "-c", "echo no > \"$0\"", "$FILE"], "extensions": [".x"]}}}"#,
        )
        .unwrap();
        assert!(list.format(&file, dir.path(), dir.path()).await);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "X");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("where"))
                .unwrap()
                .trim(),
            dir.path().canonicalize().unwrap().to_string_lossy()
        );
        assert!(
            !list
                .format(&dir.path().join("b.y"), dir.path(), dir.path())
                .await
        );
    }

    #[test]
    fn npm_formatters_need_the_dependency_and_the_bin() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path();
        let sub = project.join("src");
        std::fs::create_dir_all(sub.join("node_modules/.bin")).unwrap();
        std::fs::write(sub.join("node_modules/.bin/prettier"), "").unwrap();
        let place = Place {
            directory: &sub,
            worktree: project,
            bin: Path::new("/nonexistent"),
        };
        let prettier = Detect::Npm {
            package: "prettier",
            bin: "prettier",
            args: &["--write"],
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        assert_eq!(runtime.block_on(detect(&prettier, &place)), None);
        std::fs::write(
            project.join("package.json"),
            r#"{"devDependencies": {"prettier": "3"}}"#,
        )
        .unwrap();
        let bin = sub
            .join("node_modules/.bin")
            .join("prettier")
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            runtime.block_on(detect(&prettier, &place)),
            Some(vec![bin, "--write".into(), "$FILE".into()])
        );
    }
}
