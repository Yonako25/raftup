//! Tests des dossiers de Raftup.

use launcher_core::paths::Paths;

#[test]
fn under_donne_la_bonne_arborescence() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let p = Paths::under(root);

    assert_eq!(p.config_dir, root.join("config"));
    assert_eq!(p.data_dir, root.join("data"));
    assert_eq!(p.cache_dir, root.join("data").join("cache"));
    assert_eq!(p.config_file(), root.join("config").join("config.toml"));
    assert_eq!(p.installs_dir(), root.join("data").join("installs"));
    assert_eq!(
        p.backups_dir("slime-rancher"),
        root.join("data").join("backups").join("slime-rancher")
    );
    assert_eq!(
        p.downloads_dir(),
        root.join("data").join("cache").join("downloads")
    );
    assert_eq!(
        p.worlds_dir("0.6.1"),
        root.join("data").join("worlds").join("0.6.1")
    );
}

#[test]
fn from_os_se_termine_par_raftup() {
    let p = Paths::from_os().unwrap();
    assert!(p.config_dir.ends_with("Raftup"));
    assert!(p.data_dir.ends_with("Raftup"));
    assert!(p.cache_dir.starts_with(&p.data_dir));
}
