//! Path arithmetic for linked files, without touching the file system.

use std::path::{Component, Path, PathBuf};

/// `p` made absolute against the current directory and normalised lexically (`.` and
/// `..` removed), without touching the file system. Where there is no current directory
/// (the browser) `p` is only normalised.
pub fn absolute(p: &Path) -> PathBuf {
    let joined = if p.is_absolute() {
        p.to_owned()
    } else {
        std::env::current_dir().unwrap_or_default().join(p)
    };
    let mut out = PathBuf::new();
    for c in joined.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push(c);
                }
            }
            _ => out.push(c),
        }
    }
    out
}

fn unrooted(p: &Path) -> bool {
    !p.has_root() && !matches!(p.components().next(), Some(Component::Prefix(_)))
}

/// `target` relative to the directory `dir`, `/`-separated so project files move
/// between systems; `target` itself when they share no root (another Windows drive).
pub fn relative(dir: &Path, target: &Path) -> PathBuf {
    let (dir, target) = (absolute(dir), absolute(target));
    let a: Vec<_> = dir.components().collect();
    let b: Vec<_> = target.components().collect();
    let common = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    // Two paths with no current directory to anchor them (the browser) share a root.
    let both_relative = unrooted(&dir) && unrooted(&target);
    if !both_relative && (common == 0 || (common == 1 && matches!(a[0], Component::Prefix(_)))) {
        return target;
    }
    let parts: Vec<String> = std::iter::repeat_n("..".to_owned(), a.len() - common)
        .chain(
            b[common..]
                .iter()
                .map(|c| c.as_os_str().to_string_lossy().into_owned()),
        )
        .collect();
    PathBuf::from(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_walk_up_and_down() {
        let root = absolute(Path::new("/proj"));
        assert_eq!(
            relative(&root.join("art"), &root.join("src/title.png")),
            PathBuf::from("../src/title.png")
        );
        assert_eq!(
            relative(&root, &root.join("a/b.png")),
            PathBuf::from("a/b.png")
        );
        assert_eq!(
            relative(&root.join("x/./y/.."), &root.join("x/c.png")),
            PathBuf::from("c.png")
        );
    }
}
