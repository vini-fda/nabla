//! User-selected TOML files, using native dialogs or browser upload/download.

use crate::world::{MAX_SAVE_BYTES, World};
use std::sync::mpsc::{self, Receiver};

pub type FileResult = Result<Option<World>, String>;

pub fn load(ctx: &egui::Context) -> Receiver<FileResult> {
    let (tx, rx) = mpsc::channel();
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::io::Read as _;
        let result = rfd::FileDialog::new()
            .add_filter("World", &["toml"])
            .pick_file()
            .map(|path| {
                let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
                let mut text = String::new();
                file.take(MAX_SAVE_BYTES as u64 + 1)
                    .read_to_string(&mut text)
                    .map_err(|e| e.to_string())?;
                World::from_toml(&text)
            })
            .transpose();
        drop(tx.send(result));
        ctx.request_repaint();
    }
    #[cfg(target_arch = "wasm32")]
    {
        let ctx = ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let result = if let Some(file) = rfd::AsyncFileDialog::new()
                .add_filter("World", &["toml"])
                .pick_file()
                .await
            {
                let bytes = file.read().await;
                if bytes.len() > MAX_SAVE_BYTES {
                    Err("World saves must be at most 1 MiB".to_owned())
                } else {
                    std::str::from_utf8(&bytes)
                        .map_err(|e| e.to_string())
                        .and_then(World::from_toml)
                        .map(Some)
                }
            } else {
                Ok(None)
            };
            drop(tx.send(result));
            ctx.request_repaint();
        });
    }
    rx
}

pub fn save(ctx: &egui::Context, text: String) -> Receiver<FileResult> {
    let (tx, rx) = mpsc::channel();
    #[cfg(not(target_arch = "wasm32"))]
    {
        let result = rfd::FileDialog::new()
            .add_filter("World", &["toml"])
            .set_file_name("world.toml")
            .save_file()
            .map_or(Ok(None), |path| {
                std::fs::write(path, text)
                    .map(|()| None)
                    .map_err(|e| e.to_string())
            });
        drop(tx.send(result));
        ctx.request_repaint();
    }
    #[cfg(target_arch = "wasm32")]
    {
        let ctx = ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let result = if let Some(file) = rfd::AsyncFileDialog::new()
                .set_file_name("world.toml")
                .save_file()
                .await
            {
                file.write(text.as_bytes())
                    .await
                    .map(|()| None)
                    .map_err(|e| e.to_string())
            } else {
                Ok(None)
            };
            drop(tx.send(result));
            ctx.request_repaint();
        });
    }
    rx
}
