#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]
use qm_manager::QuarterMasterPaths;
use std::io::Write;
use std::path::PathBuf;

pub struct Fixture {
    pub tmp: tempfile::TempDir,
    pub paths: QuarterMasterPaths,
    pub game_dir: PathBuf,
    pub game_exe: PathBuf,
    pub save_dir: PathBuf,
}

impl Fixture {
    pub fn new() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let game_dir = root.join("game");
        let save_dir = root.join("save");
        std::fs::create_dir_all(&game_dir).unwrap();
        std::fs::create_dir_all(&save_dir).unwrap();
        let workspace_dir = root.join("workspace");
        std::fs::create_dir_all(&workspace_dir).unwrap();

        let game_exe = game_dir.join("mnb.exe");
        let file = std::fs::File::create(&game_exe).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default();
        zip.start_file("main.lua", opts).unwrap();
        zip.write_all(
            b"gameVersion = \"0.5.3b\"\nversion = \"0.5.3b\"\nlove.audio.play(snd_halt)\n",
        )
        .unwrap();
        zip.start_file("conf.lua", opts).unwrap();
        zip.write_all(b"t.console = false\n").unwrap();
        zip.finish().unwrap();

        let paths = QuarterMasterPaths::resolve(Some(&root.join("data")));
        // Settings default to the real game install; overwrite before anything reads them.
        let mut settings = paths.load_settings();
        settings.game_exe_path = Some(game_exe.clone());
        settings.save_dir_path = Some(save_dir.clone());
        settings.workspace_mods_dir = Some(workspace_dir);
        settings.first_run_completed = true;
        paths.save_settings(&settings).unwrap();

        Fixture {
            tmp,
            paths,
            game_dir,
            game_exe,
            save_dir,
        }
    }

    pub fn add_mod(&self, id: &str, version: &str, qm_toml: &str) -> PathBuf {
        let dir = self
            .tmp
            .path()
            .join("src_mods")
            .join(format!("{id}-{version}"));
        std::fs::create_dir_all(&dir).unwrap();
        let mod_json = serde_json::json!({ "id": id, "name": id, "version": version });
        std::fs::write(
            dir.join("mod.json"),
            serde_json::to_string_pretty(&mod_json).unwrap(),
        )
        .unwrap();
        std::fs::write(dir.join("qm.toml"), qm_toml).unwrap();
        self.paths.import_mod_from_path(&dir, None, false).unwrap();
        dir
    }

    pub fn write_dll(&self, bytes: &[u8]) {
        std::fs::write(self.game_dir.join("winmm.dll"), bytes).unwrap();
    }
}
