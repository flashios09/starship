use nix::NixPath;
use std::fs;
use std::path::{Component, Path};
use unicode_segmentation::UnicodeSegmentation;

use super::directory::{convert_path_sep, is_readonly_dir};
use super::{Context, Module};

use crate::config::ModuleConfig;
use crate::configs::smart_directory::SmartDirectoryConfig;
use crate::formatter::StringFormatter;

/// Creates a module with the current logical or physical directory
///
/// TODO: update module docblock
///
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("smart_directory");
    let config: SmartDirectoryConfig = SmartDirectoryConfig::try_load(module.config);

    let home_dir = context
        .get_home()
        .expect("Unable to determine HOME_DIR for user");
    let physical_dir = &context.current_dir;
    let display_dir = if config.use_logical_path {
        &context.logical_dir
    } else {
        &context.current_dir
    };

    log::debug!("Home dir: {:?}", &home_dir);
    log::debug!("Physical dir: {:?}", &physical_dir);
    log::debug!("Display dir: {:?}", &display_dir);

    let repo = context.get_repo().ok();

    let path_vec = match &repo.and_then(|r| r.workdir.as_ref()) {
        Some(repo_root) => {
            let before =
                truncate_before_root_dir(repo_root.parent()?, &home_dir, config.home_symbol);

            let root = repo_root.file_name().unwrap().to_string_lossy().to_string();

            let after_repo_root = truncate_after_repo_root(display_dir, repo_root);

            [before, root, after_repo_root]
        }
        _ => [
            String::new(),
            String::new(),
            truncate(display_dir, &home_dir, config.home_symbol),
        ],
    };

    let path_vec = if config.use_os_path_sep {
        path_vec.map(|i| convert_path_sep(&i))
    } else {
        path_vec
    };

    let display_format = if path_vec[0].is_empty() && path_vec[1].is_empty() {
        config.format
    } else {
        config.repo_root_format
    };
    let repo_root_style = config.repo_root_style.unwrap_or(config.style);
    let before_repo_root_style = config.before_repo_root_style.unwrap_or(config.style);
    let after_repo_root_style = config.after_repo_root_style.unwrap_or(config.style);

    let parsed = StringFormatter::new(display_format).and_then(|formatter| {
        formatter
            .map_style(|variable| match variable {
                "style" => Some(Ok(config.style)),
                "read_only_style" => Some(Ok(config.read_only_style)),
                "before_repo_root_style" => Some(Ok(before_repo_root_style)),
                "repo_root_style" => Some(Ok(repo_root_style)),
                "after_repo_root_style" => Some(Ok(after_repo_root_style)),
                _ => None,
            })
            .map(|variable| match variable {
                "path" => Some(Ok(path_vec[2].as_str())),
                "before_root_path" => Some(Ok(path_vec[0].as_str())),
                "repo_root" => Some(Ok(path_vec[1].as_str())),
                "after_root_path" => Some(Ok(path_vec[2].as_str())),
                "read_only" => {
                    if is_readonly_dir(physical_dir) {
                        Some(Ok(config.read_only))
                    } else {
                        None
                    }
                }
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `smart_directory`:\n{error}");
            return None;
        }
    });

    Some(module)
}

fn truncate_before_root_dir(path: &Path, home: &Path, home_symbol: &str) -> String {
    if path.is_empty() {
        return String::new();
    }

    let dirs = get_path_dirs(path);

    let mut current_dir = String::new();
    let mut before_root_dir = String::new();
    for dir in dirs {
        current_dir = current_dir + "/" + &dir;

        before_root_dir += "/";
        let part = if has_sibling_collision(Path::new(&current_dir)) {
            dir
        } else {
            shorten_dir(&dir)
        };

        before_root_dir += &part;
    }

    if path.starts_with(home) {
        before_root_dir = before_root_dir.replacen(&shorten_path(home), home_symbol, 1);
    }

    if before_root_dir != "/" {
        before_root_dir += "/";
    }

    before_root_dir
}

fn truncate_after_repo_root(path: &Path, repo_root: &Path) -> String {
    if path.strip_prefix(repo_root).unwrap().is_empty() {
        return String::new();
    }

    let dirs = get_path_dirs(path);
    let repo_root_length = get_path_dirs(repo_root).len();

    let mut current_dir = String::new();
    let mut after_repo_root = String::new();
    let current_dir_index = dirs.len() - 1;
    for (i, dir) in dirs.iter().enumerate() {
        current_dir = current_dir + "/" + dir;

        // skip `repo_root` dirs
        if i < repo_root_length {
            continue;
        }

        after_repo_root += "/";

        // always keep the current dir(last dir) non-truncated
        if i == current_dir_index {
            after_repo_root += dir;

            break;
        }

        let part = if has_sibling_collision(Path::new(&current_dir)) {
            dir
        } else {
            &shorten_dir(dir)
        };

        after_repo_root += part;
    }

    after_repo_root
}

fn truncate(path: &Path, home: &Path, home_symbol: &str) -> String {
    if path.is_empty() {
        return String::new();
    }

    if path == home {
        return home_symbol.to_string();
    }

    let dirs = get_path_dirs(path);

    let mut current_dir = String::new();
    let mut truncated_path = String::new();
    for (i, dir) in dirs.iter().enumerate() {
        // we have reached the last dir: append it and quit the loop
        if i == dirs.len() - 1 {
            truncated_path = truncated_path + "/" + dir;
            break;
        }

        current_dir = current_dir + "/" + dir;

        truncated_path += "/";
        let part = if has_sibling_collision(Path::new(&current_dir)) {
            dir
        } else {
            &shorten_dir(dir)
        };

        truncated_path += part;
    }

    if path.starts_with(home) {
        truncated_path = truncated_path.replacen(&shorten_path(home), home_symbol, 1);
    }

    truncated_path
}

fn get_path_dirs(path: &Path) -> Vec<String> {
    let dirs: Vec<String> = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(os_str) => Some(os_str.to_string_lossy().to_string()),
            _ => None,
        })
        .collect();

    dirs
}

fn shorten_dir(dir: &str) -> String {
    let mut graphemes = dir.graphemes(true);

    if dir.starts_with('.') {
        // e.g `.config` to `.c`
        let mut hidden_dir = String::new();
        let dot = graphemes.next().unwrap();

        hidden_dir.push_str(dot);

        if let Some(next) = graphemes.next() {
            hidden_dir.push_str(next);
        }

        return hidden_dir;
    }

    // e.g. `Volumes` to `V`
    // dir.chars().next().unwrap().to_string()
    graphemes.next().unwrap().to_string()
}

fn get_siblings(current_dir: &Path) -> Vec<String> {
    let mut siblings = Vec::new();
    let dir = current_dir.file_name().unwrap().to_string_lossy();

    if let Some(parent_dir) = current_dir.parent() {
        if let Ok(entries) = fs::read_dir(parent_dir) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    if let Ok(sibling) = entry.file_name().into_string() {
                        // skip current dir
                        if sibling == dir {
                            continue;
                        }

                        siblings.push(sibling);
                    }
                }
            }
        }
    }

    siblings
}

fn has_sibling_collision(current_dir: &Path) -> bool {
    let siblings = get_siblings(current_dir);

    if siblings.is_empty() {
        return false;
    }

    let shorten_current = shorten_dir(&current_dir.file_name().unwrap().to_string_lossy());
    let mut has_sibling_collision = false;

    for sibling in siblings {
        let shorten_sibling = shorten_dir(&sibling);

        if shorten_sibling == shorten_current {
            has_sibling_collision = true;

            break;
        }
    }

    has_sibling_collision
}

fn shorten_path(path: &Path) -> String {
    if path.is_empty() {
        return String::new();
    }

    if path.to_string_lossy() == "/" {
        return "/".to_string();
    }

    let dirs = get_path_dirs(path);

    let mut current_dir = String::new();
    let mut shorten_path = String::new();
    for dir in dirs {
        current_dir = current_dir + "/" + &dir;

        shorten_path += "/";
        let part = if has_sibling_collision(Path::new(&current_dir)) {
            dir
        } else {
            shorten_dir(&dir)
        };

        shorten_path += &part;
    }

    shorten_path
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test::ModuleRenderer;
    use crate::utils::create_command;
    use nu_ansi_term::Color;
    use std::env;
    use std::path::PathBuf;
    use std::{fs, io};

    fn starship_tempdir(namespace: &str) -> PathBuf {
        let tmp = env::temp_dir();
        let starship_tempdir = tmp.join("startship_test").join(namespace);

        // remove `/path_to_os_tmp_dir/startship_test` dir if it was already created in a previous test
        let _ = fs::remove_dir_all(&starship_tempdir);

        // create a new empty `/path_to_os_tmp_dir/starship_test` dir
        fs::create_dir_all(&starship_tempdir).unwrap();

        starship_tempdir
    }

    #[test]
    fn it_starship_tempdir() {
        let starship_tempdir = starship_tempdir("/");

        assert!(starship_tempdir.exists());
        assert!(starship_tempdir.is_dir());

        // let entries = fs::read_dir(&starship_tempdir).unwrap().count();
        // assert_eq!(entries, 0);
    }

    #[test]
    fn it_shorten_dir() {
        // normal dir
        assert_eq!(shorten_dir("starship"), "s");
        assert_eq!(shorten_dir("Lab"), "L");

        // hidden dir
        assert_eq!(shorten_dir(".config"), ".c");

        // unicode dir
        assert_eq!(shorten_dir("目录"), "目");
        assert_eq!(shorten_dir("フォルダ"), "フ");
        assert_eq!(shorten_dir("한글"), "한");
        assert_eq!(shorten_dir("a̐éö̲"), "a̐");

        // single char dir
        assert_eq!(shorten_dir("s"), "s");
        assert_eq!(shorten_dir("L"), "L");
        assert_eq!(shorten_dir("目"), "目");
        assert_eq!(shorten_dir("フ"), "フ");
        assert_eq!(shorten_dir("한"), "한");
        assert_eq!(shorten_dir("a̐"), "a̐");
        assert_eq!(shorten_dir("."), ".");
    }

    #[test]
    fn it_get_path_dirs() {
        // one level path
        assert_eq!(get_path_dirs(Path::new("/home")), vec!["home"]);

        // nested path
        assert_eq!(get_path_dirs(Path::new("/home/me")), vec!["home", "me"]);
        assert_eq!(
            get_path_dirs(Path::new("/Volumes/Data/Lab")),
            vec!["Volumes", "Data", "Lab"]
        );

        // with hidden dir
        assert_eq!(
            get_path_dirs(Path::new("/home/me/.config/starship")),
            vec!["home", "me", ".config", "starship"]
        );

        // path with unicode dir
        assert_eq!(
            get_path_dirs(Path::new("/home/me/目录")),
            vec!["home", "me", "目录"]
        );
        assert_eq!(
            get_path_dirs(Path::new("/home/me/フォルダ")),
            vec!["home", "me", "フォルダ"]
        );
        assert_eq!(
            get_path_dirs(Path::new("/home/me/한글")),
            vec!["home", "me", "한글"]
        );
        assert_eq!(
            get_path_dirs(Path::new("/home/me/a̐éö̲")),
            vec!["home", "me", "a̐éö̲"]
        );

        //  with mutltiple unicode dirs
        assert_eq!(
            get_path_dirs(Path::new("/目录/フォルダ/한글/a̐éö̲")),
            ["目录", "フォルダ", "한글", "a̐éö̲"]
        );

        // with trailing slash
        assert_eq!(
            get_path_dirs(Path::new("/Volumes/Data/Lab/")),
            vec!["Volumes", "Data", "Lab"]
        );

        // PS: root will return an empty vector
        assert_eq!(get_path_dirs(Path::new("/")), Vec::<String>::new());
    }

    #[test]
    fn it_shorten_path() {
        let starship_tempdir = starship_tempdir("shorten_path");
        let prefix = shorten_path(&starship_tempdir);

        // normal path
        assert_eq!(shorten_path(Path::new("/")), "/");
        assert_eq!(shorten_path(Path::new("/home")), "/h");
        assert_eq!(shorten_path(Path::new("/home/me")), "/h/m");

        // path with hidden dir
        assert_eq!(shorten_path(Path::new("/home/me/.config")), "/h/m/.c");

        // path with unicode dir
        assert_eq!(shorten_path(Path::new("/home/me/目录")), "/h/m/目");
        assert_eq!(shorten_path(Path::new("/home/me/フォルダ")), "/h/m/フ");
        assert_eq!(shorten_path(Path::new("/home/me/한글")), "/h/m/한");
        assert_eq!(shorten_path(Path::new("/home/me/a̐éö̲")), "/h/m/a̐");

        // path with mutltiple unicode dirs
        assert_eq!(
            shorten_path(Path::new("/目录/フォルダ/한글/a̐éö̲")),
            "/目/フ/한/a̐"
        );

        // path with trailing slash
        assert_eq!(shorten_path(Path::new("/with/trailing/slash/")), "/w/t/s");

        // PS: NO SUPPORT FOR $HOME path, handled separately !
        assert_ne!(shorten_path(Path::new("~/.config/starship")), "~/.c/s");
        assert_eq!(shorten_path(Path::new("~/.config/starship")), "/~/.c/s");

        let _ = fs::create_dir_all(starship_tempdir.join("home/me/.local/share"));
        let _ = fs::create_dir_all(starship_tempdir.join("home/me/.local/state"));

        // with sibling collision
        assert_eq!(
            shorten_path(&starship_tempdir.join("home/me/.local/share")),
            prefix + "/h/m/.l/share"
        );
    }

    #[test]
    fn it_get_siblings() {
        let starship_tempdir = starship_tempdir("get_siblings");

        let _ = fs::create_dir_all(starship_tempdir.join("home/me/.local/share"));
        let _ = fs::create_dir_all(starship_tempdir.join("home/me/.local/state"));
        let share_siblings = get_siblings(&starship_tempdir.join("home/me/.local/share"));

        assert_eq!(share_siblings, vec!["state"]);

        let _ = fs::create_dir_all(starship_tempdir.join("home/me/.config/borders"));
        let _ = fs::create_dir_all(starship_tempdir.join("home/me/.config/ghostty"));
        let _ = fs::create_dir_all(starship_tempdir.join("home/me/.config/nvim"));
        let _ = fs::create_dir_all(starship_tempdir.join("home/me/.config/starship"));

        let mut nvim_siblings = get_siblings(&starship_tempdir.join("home/me/.config/nvim"));
        // always sort the siblings since we don't know the `fs::read_dir` read order
        nvim_siblings.sort();

        assert_eq!(nvim_siblings, vec!["borders", "ghostty", "starship"]);

        let dot_config_siblings = get_siblings(&starship_tempdir.join("home/me/.config"));

        assert_eq!(dot_config_siblings, vec![".local"]);

        let _ = fs::create_dir_all(starship_tempdir.join("home/me/unicode/目录"));
        let _ = fs::create_dir_all(starship_tempdir.join("home/me/unicode/フォルダ"));
        let _ = fs::create_dir_all(starship_tempdir.join("home/me/unicode/한글"));
        let _ = fs::create_dir_all(starship_tempdir.join("home/me/unicode/a̐éö̲"));

        let mut unicode_dir_siblings = get_siblings(&starship_tempdir.join("home/me/unicode/目录"));
        unicode_dir_siblings.sort();

        assert_eq!(unicode_dir_siblings, vec!["a̐éö̲", "フォルダ", "한글"]);

        let _ = fs::create_dir_all(starship_tempdir.join("Volumes/Data/Lab"));

        let lab_siblings = get_siblings(&starship_tempdir.join("Volumes/Data/Lab"));

        assert_eq!(lab_siblings, Vec::<String>::new());

        let home_siblings = get_siblings(&starship_tempdir.join("home/me"));

        assert_eq!(home_siblings, Vec::<String>::new());
    }

    #[test]
    fn it_has_sibling_collision() {
        let starship_tempdir = starship_tempdir("has_sibling_collision");

        let _ = fs::create_dir_all(starship_tempdir.join("home/me/.local/share"));
        let _ = fs::create_dir_all(starship_tempdir.join("home/me/.local/state"));

        // sibling collision for `share` since `state` dir starts with `s` too !
        assert!(has_sibling_collision(
            &starship_tempdir.join("home/me/.local/share")
        ));

        let _ = fs::create_dir_all(starship_tempdir.join("home/me/.config/borders"));
        let _ = fs::create_dir_all(starship_tempdir.join("home/me/.config/ghostty"));
        let _ = fs::create_dir_all(starship_tempdir.join("home/me/.config/nvim"));
        let _ = fs::create_dir_all(starship_tempdir.join("home/me/.config/starship"));

        // no sibling collision for `nvim` since no other dirs starts with `n`
        assert!(!has_sibling_collision(
            &starship_tempdir.join("home/me/.config/nvim")
        ));

        let _ = fs::create_dir_all(starship_tempdir.join("home/me/unicode/目录"));
        let _ = fs::create_dir_all(starship_tempdir.join("home/me/unicode/フォルダ"));
        let _ = fs::create_dir_all(starship_tempdir.join("home/me/unicode/한글"));
        let _ = fs::create_dir_all(starship_tempdir.join("home/me/unicode/a̐éö̲"));

        // no sibling collision for `フォルダ` since no other dirs starts with `フ`
        assert!(!has_sibling_collision(
            &starship_tempdir.join("home/me/unicode/フォルダ")
        ));

        let _ = fs::create_dir_all(starship_tempdir.join("home/me/unicode/フルダォ"));

        // sibling collision for `フォルダ` since other dir starts with `フ`
        assert!(has_sibling_collision(
            &starship_tempdir.join("home/me/unicode/フォルダ")
        ));

        let _ = fs::create_dir_all(starship_tempdir.join("home/me/.cache"));

        // sibling collision for `.config` since `.cache` dir starts with `.c` too !
        assert!(has_sibling_collision(
            &starship_tempdir.join("home/me/.config")
        ));
    }

    #[test]
    fn it_truncate_before_root_dir() {
        let starship_tempdir = starship_tempdir("truncate_before_root_dir");
        let prefix = shorten_path(&starship_tempdir);

        let _ = fs::create_dir_all(starship_tempdir.join("home/me/.local/share"));
        let _ = fs::create_dir_all(starship_tempdir.join("home/me/.local/state"));

        let home = starship_tempdir.join("home/me");

        // before root dir is `$HOME`
        assert_eq!(
            truncate_before_root_dir(&starship_tempdir.join("home/me"), &home, "~"),
            "~/"
        );

        // use an emoji as home_symbol
        assert_eq!(
            truncate_before_root_dir(&starship_tempdir.join("home/me"), &home, "🏠"),
            "🏠/"
        );

        // hidden dir
        assert_eq!(
            truncate_before_root_dir(&starship_tempdir.join("home/me/.local"), &home, "~"),
            "~/.l/"
        );

        // sibling collision: `share` and `state` both starts with `s`
        assert_eq!(
            truncate_before_root_dir(&starship_tempdir.join("home/me/.local/share"), &home, "~"),
            "~/.l/share/"
        );

        let _ = fs::create_dir_all(starship_tempdir.join("home/me/unicode/目录"));
        let _ = fs::create_dir_all(starship_tempdir.join("home/me/unicode/フォルダ"));
        let _ = fs::create_dir_all(starship_tempdir.join("home/me/unicode/한글"));
        let _ = fs::create_dir_all(starship_tempdir.join("home/me/unicode/a̐éö̲"));

        // unicode
        assert_eq!(
            truncate_before_root_dir(&starship_tempdir.join("home/me/unicode/目录"), &home, "~"),
            "~/u/目/"
        );

        // unicode
        assert_eq!(
            truncate_before_root_dir(
                &starship_tempdir.join("home/me/unicode/フォルダ"),
                &home,
                "~"
            ),
            "~/u/フ/"
        );

        // unicode
        assert_eq!(
            truncate_before_root_dir(&starship_tempdir.join("home/me/unicode/한글"), &home, "~"),
            "~/u/한/"
        );

        // unicode
        assert_eq!(
            truncate_before_root_dir(&starship_tempdir.join("home/me/unicode/a̐éö̲"), &home, "~"),
            "~/u/a̐/"
        );

        let _ = fs::create_dir_all(starship_tempdir.join("home/me/unicode/フルダォ"));

        // unicode with sibling collision
        assert_eq!(
            truncate_before_root_dir(
                &starship_tempdir.join("home/me/unicode/フルダォ"),
                &home,
                "~"
            ),
            "~/u/フルダォ/"
        );

        let _ = fs::create_dir_all(starship_tempdir.join("Volumes/Data/Lab"));

        // normal dir(outside of `$HOME`)
        assert_eq!(
            truncate_before_root_dir(&starship_tempdir.join("Volumes/Data/Lab"), &home, "~"),
            prefix + "/V/D/L/"
        );
    }

    #[test]
    fn it_truncate() {
        let starship_tempdir = starship_tempdir("truncate_before_root_dir");
        let prefix = shorten_path(&starship_tempdir);

        let _ = fs::create_dir_all(starship_tempdir.join("home/me/.local/share"));
        let _ = fs::create_dir_all(starship_tempdir.join("home/me/.local/share/nvim"));
        let _ = fs::create_dir_all(starship_tempdir.join("home/me/.local/state"));
        let _ = fs::create_dir_all(starship_tempdir.join("home/me/.local/state/nvim"));

        let home = starship_tempdir.join("home/me");

        // truncate `$HOME`
        assert_eq!(truncate(&starship_tempdir.join("home/me"), &home, "~"), "~");

        // truncate `$HOME` with emoji used as `home_symbol`
        assert_eq!(
            truncate(&starship_tempdir.join("home/me"), &home, "🏠"),
            "🏠"
        );

        // hidden dir
        assert_eq!(
            truncate(&starship_tempdir.join("home/me/.local"), &home, "~"),
            "~/.local"
        );

        // sibling collision: `share` and `state` both starts with `s`
        assert_eq!(
            truncate(
                &starship_tempdir.join("home/me/.local/state/nvim"),
                &home,
                "~"
            ),
            "~/.l/state/nvim"
        );

        let _ = fs::create_dir_all(starship_tempdir.join("home/me/unicode/目录"));
        let _ = fs::create_dir_all(starship_tempdir.join("home/me/unicode/フォルダ"));
        let _ = fs::create_dir_all(starship_tempdir.join("home/me/unicode/한글"));
        let _ = fs::create_dir_all(starship_tempdir.join("home/me/unicode/a̐éö̲"));

        // unicode
        assert_eq!(
            truncate(&starship_tempdir.join("home/me/unicode/目录"), &home, "~"),
            "~/u/目录"
        );

        // unicode
        assert_eq!(
            truncate(
                &starship_tempdir.join("home/me/unicode/フォルダ"),
                &home,
                "~"
            ),
            "~/u/フォルダ"
        );

        // unicode
        assert_eq!(
            truncate(&starship_tempdir.join("home/me/unicode/한글"), &home, "~"),
            "~/u/한글"
        );

        // unicode
        assert_eq!(
            truncate(&starship_tempdir.join("home/me/unicode/a̐éö̲"), &home, "~"),
            "~/u/a̐éö̲"
        );

        let _ = fs::create_dir_all(starship_tempdir.join("Volumes/Data/Lab"));

        // normal dir(outside of `$HOME`)
        assert_eq!(
            truncate(&starship_tempdir.join("Volumes/Data/Lab"), &home, "~"),
            prefix + "/V/D/Lab"
        );
    }

    #[test]
    fn it_truncate_after_repo_root() {
        let starship_tempdir = starship_tempdir("truncate_after_repo_root");
        // let prefix = shorten_path(&starship_tempdir);
        let mut repo_root = starship_tempdir.join("home/me/.config/nvim");

        // create the git root dirs
        let _ = fs::create_dir_all(&repo_root);
        let _ = fs::create_dir_all(repo_root.join("lua"));

        // with git root sub-directory `lua`
        assert_eq!(
            truncate_after_repo_root(&repo_root.join("lua"), &repo_root),
            "/lua"
        );

        let _ = fs::create_dir_all(repo_root.join("lua/plugins"));

        // with git root sub-directories `lua/plugins`
        assert_eq!(
            truncate_after_repo_root(&repo_root.join("lua/plugins"), &repo_root),
            "/l/plugins"
        );

        let _ = fs::create_dir_all(repo_root.join("lsp"));

        // sibling collision since `lua` and `lsp` starts with `l`
        assert_eq!(
            truncate_after_repo_root(&repo_root.join("lua/plugins"), &repo_root),
            "/lua/plugins"
        );

        // use `home/me/unicode` as `repo_root`
        repo_root = starship_tempdir.join("home/me/unicode");

        let _ = fs::create_dir_all(starship_tempdir.join("home/me/unicode/目录"));
        let _ = fs::create_dir_all(starship_tempdir.join("home/me/unicode/フォルダ"));
        let _ = fs::create_dir_all(starship_tempdir.join("home/me/unicode/한글"));
        let _ = fs::create_dir_all(starship_tempdir.join("home/me/unicode/a̐éö̲"));

        // unicode
        assert_eq!(
            truncate_after_repo_root(&repo_root.join("目录"), &repo_root),
            "/目录"
        );

        // unicode
        assert_eq!(
            truncate_after_repo_root(&repo_root.join("フォルダ"), &repo_root),
            "/フォルダ"
        );

        // unicode
        assert_eq!(
            truncate_after_repo_root(&repo_root.join("한글"), &repo_root),
            "/한글"
        );

        // unicode
        assert_eq!(
            truncate_after_repo_root(&repo_root.join("a̐éö̲"), &repo_root),
            "/a̐éö̲"
        );

        let _ = fs::create_dir_all(starship_tempdir.join("home/me/unicode/目录/フォルダ/한글/a̐éö̲"));

        // nested unicode
        assert_eq!(
            truncate_after_repo_root(&repo_root.join("目录/フォルダ/한글/a̐éö̲"), &repo_root),
            "/目/フ/한/a̐éö̲"
        );

        let _ = fs::create_dir_all(starship_tempdir.join("home/me/unicode/目录/フルダォ"));

        // nested unicode with sibling collision
        assert_eq!(
            truncate_after_repo_root(&repo_root.join("目录/フォルダ/한글/a̐éö̲"), &repo_root),
            "/目/フォルダ/한/a̐éö̲"
        );
    }

    fn init_repo(path: &Path) -> io::Result<()> {
        create_command("git")?
            .args(["init"])
            .current_dir(path)
            .output()
            .map(|_| ())
    }

    #[test]
    fn it_module() {
        let starship_tempdir = starship_tempdir("module");
        let prefix = shorten_path(&starship_tempdir);
        let home = starship_tempdir.join("home/me");
        let _ = fs::create_dir_all(&home);

        // in `$HOME` directory
        assert_eq!(
            // module
            ModuleRenderer::new("smart_directory")
                .path(&home)
                .env("HOME", home.to_string_lossy())
                .collect(),
            // output
            Some(format!("{} ", Color::Cyan.bold().paint("~")))
        );

        // `config.home_symbol`
        assert_eq!(
            // module
            ModuleRenderer::new("smart_directory")
                .path(&home)
                .env("HOME", home.to_string_lossy())
                .config(toml::toml! {
                    [smart_directory]
                    home_symbol = "🏠"
                })
                .collect(),
            // output
            Some(format!("{} ", Color::Cyan.bold().paint("🏠")))
        );

        // `read-only` path
        assert_eq!(
            // module
            ModuleRenderer::new("smart_directory")
                .path(Path::new("/etc"))
                .collect(),
            // output
            Some(format!(
                "{}{} ",
                Color::Cyan.bold().paint("/etc"),
                Color::Red.paint("🔒")
            ))
        );

        // `config.read_only`
        assert_eq!(
            // module
            ModuleRenderer::new("smart_directory")
                .path(Path::new("/etc"))
                .config(toml::toml! {
                    [smart_directory]
                    read_only = "RO"
                })
                .collect(),
            // output
            Some(format!(
                "{}{} ",
                Color::Cyan.bold().paint("/etc"),
                Color::Red.paint("RO")
            ))
        );

        // `config.read_only_style`
        assert_eq!(
            // module
            ModuleRenderer::new("smart_directory")
                .path(Path::new("/etc"))
                .config(toml::toml! {
                    [smart_directory]
                    read_only_style = "yellow"
                })
                .collect(),
            // output
            Some(format!(
                "{}{} ",
                Color::Cyan.bold().paint("/etc"),
                Color::Yellow.paint("🔒")
            ))
        );

        let _ = fs::create_dir_all(starship_tempdir.join("Volumes/Data/Lab"));

        // outside of `$HOME` directory
        assert_eq!(
            // module
            ModuleRenderer::new("smart_directory")
                .path(starship_tempdir.join("Volumes/Data/Lab"))
                .env("HOME", home.to_string_lossy())
                .collect(),
            // output
            Some(format!(
                "{} ",
                Color::Cyan.bold().paint(prefix.clone() + "/V/D/Lab")
            ))
        );

        // `config.style`
        assert_eq!(
            // module
            ModuleRenderer::new("smart_directory")
                .path(starship_tempdir.join("Volumes/Data/Lab"))
                .env("HOME", home.to_string_lossy())
                .config(toml::toml! {
                    [smart_directory]
                    style = "blue"
                })
                .collect(),
            // output
            Some(format!(
                "{} ",
                Color::Blue.paint(prefix.clone() + "/V/D/Lab")
            ))
        );

        // `config.format`
        assert_eq!(
            // module
            ModuleRenderer::new("smart_directory")
                .path(starship_tempdir.join("Volumes/Data/Lab"))
                .env("HOME", home.to_string_lossy())
                .config(toml::toml! {
                    [smart_directory]
                    format = "at [$path]($style)[$read_only]($read_only_style) "
                })
                .collect(),
            // output
            Some(format!(
                "at {} ",
                Color::Cyan.bold().paint(prefix.clone() + "/V/D/Lab")
            ))
        );

        let _ =
            fs::create_dir_all(starship_tempdir.join("Volumes/Data/Lab/starship/src/modules/unix"));
        let _ = fs::create_dir_all(
            starship_tempdir.join("Volumes/Data/Lab/starship/src/modules/windows"),
        );
        // `git init`
        let _ = init_repo(&starship_tempdir.join("Volumes/Data/Lab/starship"));

        // git
        assert_eq!(
            // module
            ModuleRenderer::new("smart_directory")
                .path(starship_tempdir.join("Volumes/Data/Lab/starship"))
                .env("HOME", home.to_string_lossy())
                .collect(),
            // output
            Some(format!(
                "{} ",
                // before, repo_root
                Color::Cyan
                    .bold()
                    .paint(prefix.clone() + "/V/D/L/" + "starship")
            ))
        );
        assert_eq!(
            // module
            ModuleRenderer::new("smart_directory")
                .path(starship_tempdir.join("Volumes/Data/Lab/starship/src"))
                .env("HOME", home.to_string_lossy())
                .collect(),
            // output
            Some(format!(
                "{} ",
                Color::Cyan
                    .bold()
                    // before, repo_root, after_repo_root
                    .paint(prefix.clone() + "/V/D/L/" + "starship" + "/src")
            ))
        );
        assert_eq!(
            // module
            ModuleRenderer::new("smart_directory")
                .path(starship_tempdir.join("Volumes/Data/Lab/starship/src/modules"))
                .env("HOME", home.to_string_lossy())
                .collect(),
            // output
            Some(format!(
                "{} ",
                Color::Cyan
                    .bold()
                    // before, repo_root, after_repo_root
                    .paint(prefix.clone() + "/V/D/L/" + "starship" + "/s/modules")
            ))
        );
        assert_eq!(
            // module
            ModuleRenderer::new("smart_directory")
                .path(starship_tempdir.join("Volumes/Data/Lab/starship/src/modules/windows"))
                .env("HOME", home.to_string_lossy())
                .collect(),
            // output
            Some(format!(
                "{} ",
                Color::Cyan
                    .bold()
                    // before, repo_root, after_repo_root
                    .paint(prefix.clone() + "/V/D/L/" + "starship" + "/s/m/windows")
            ))
        );

        // `config.repo_root_format`
        assert_eq!(
            // module
            ModuleRenderer::new("smart_directory")
                .path(starship_tempdir.join("Volumes/Data/Lab/starship"))
                .env("HOME", home.to_string_lossy())
                .config(toml::toml! {
                    [smart_directory]
                    repo_root_format = "at [$before_root_path]($before_repo_root_style)[$repo_root]($repo_root_style)[$after_root_path]($after_repo_root_style)[$read_only]($read_only_style) "
                })
                .collect(),
            // output
            Some(format!(
                "at {} ",
                // before, repo_root
                Color::Cyan.bold().paint(prefix.clone() + "/V/D/L/" + "starship")
            ))
        );
        // `config.repo_root_style`
        assert_eq!(
            // module
            ModuleRenderer::new("smart_directory")
                .path(starship_tempdir.join("Volumes/Data/Lab/starship"))
                .env("HOME", home.to_string_lossy())
                .config(toml::toml! {
                    [smart_directory]
                    repo_root_style = "yellow"
                })
                .collect(),
            // output
            Some(format!(
                "{}{}{}{} ",
                Color::Cyan.bold().paint(prefix.clone() + "/V/D/L/"),
                Color::Yellow.prefix(),
                "starship",
                Color::Cyan.bold().paint(""),
            ))
        );
        // `config.before_repo_root_style`
        assert_eq!(
            // module
            ModuleRenderer::new("smart_directory")
                .path(starship_tempdir.join("Volumes/Data/Lab/starship"))
                .env("HOME", home.to_string_lossy())
                .config(toml::toml! {
                    [smart_directory]
                    before_repo_root_style = "yellow"
                })
                .collect(),
            // output
            Some(format!(
                "{}{}{} ",
                Color::Yellow.prefix(),
                prefix.clone() + "/V/D/L/",
                Color::Cyan.bold().paint("starship"),
            ))
        );
        // `config.after_repo_root_style`
        assert_eq!(
            // module
            ModuleRenderer::new("smart_directory")
                .path(starship_tempdir.join("Volumes/Data/Lab/starship/src/modules/windows"))
                .env("HOME", home.to_string_lossy())
                .config(toml::toml! {
                    [smart_directory]
                    after_repo_root_style = "yellow"
                })
                .collect(),
            // output
            Some(format!(
                "{}{} ",
                Color::Cyan
                    .bold()
                    .paint(prefix.clone() + "/V/D/L/" + "starship"),
                Color::Yellow.paint("/s/m/windows"),
            ))
        );
        // `config.before_repo_root_style` + `config.repo_root_style` + `config.after_repo_root_style`
        assert_eq!(
            // module
            ModuleRenderer::new("smart_directory")
                .path(starship_tempdir.join("Volumes/Data/Lab/starship/src/modules/windows"))
                .env("HOME", home.to_string_lossy())
                .config(toml::toml! {
                    [smart_directory]
                    before_repo_root_style = "yellow"
                    repo_root_style = "cyan"
                    after_repo_root_style = "blue"
                })
                .collect(),
            // output
            Some(format!(
                "{}{}{}{}{} ",
                Color::Yellow.prefix(),
                prefix.clone() + "/V/D/L/",
                Color::Cyan.prefix(),
                "starship",
                Color::Blue.paint("/s/m/windows"),
            ))
        );
    }
}
