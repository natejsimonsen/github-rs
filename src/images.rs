//! Loads `https://` images (avatars, pictures in comments) for egui, using
//! the same HTTP client as the API. Each URL is fetched once on a background
//! thread and kept in memory.

use egui::load::{Bytes, BytesLoadResult, BytesLoader, BytesPoll, LoadError};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

type Entry = Option<Result<(Arc<[u8]>, Option<String>), String>>;

pub struct HttpLoader {
    agent: ureq::Agent,
    cache: Arc<Mutex<HashMap<String, Entry>>>,
}

impl HttpLoader {
    pub fn install(ctx: &egui::Context) {
        let loader = HttpLoader { agent: crate::github::http_agent(), cache: Default::default() };
        ctx.add_bytes_loader(Arc::new(loader));
    }
}

impl BytesLoader for HttpLoader {
    fn id(&self) -> &str {
        egui::generate_loader_id!(HttpLoader)
    }

    fn load(&self, ctx: &egui::Context, uri: &str) -> BytesLoadResult {
        if !(uri.starts_with("https://") || uri.starts_with("http://")) {
            return Err(LoadError::NotSupported);
        }
        let mut cache = self.cache.lock().unwrap();
        match cache.get(uri) {
            Some(Some(Ok((bytes, mime)))) => {
                return Ok(BytesPoll::Ready { size: None, bytes: Bytes::Shared(bytes.clone()), mime: mime.clone() });
            }
            Some(Some(Err(e))) => return Err(LoadError::Loading(e.clone())),
            Some(None) => return Ok(BytesPoll::Pending { size: None }),
            None => {}
        }
        cache.insert(uri.to_string(), None);
        let (agent, cache, ctx, uri) = (self.agent.clone(), self.cache.clone(), ctx.clone(), uri.to_string());
        std::thread::spawn(move || {
            let result = (|| -> Result<(Arc<[u8]>, Option<String>), String> {
                let resp = agent.get(&uri).call().map_err(|e| e.to_string())?;
                if !resp.status().is_success() {
                    return Err(format!("HTTP {}", resp.status()));
                }
                let mime = resp
                    .headers()
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string());
                let mut body = resp.into_body();
                let bytes = body.with_config().limit(20 * 1024 * 1024).read_to_vec().map_err(|e| e.to_string())?;
                Ok((bytes.into(), mime))
            })();
            cache.lock().unwrap().insert(uri, Some(result));
            ctx.request_repaint();
        });
        Ok(BytesPoll::Pending { size: None })
    }

    fn forget(&self, uri: &str) {
        self.cache.lock().unwrap().remove(uri);
    }

    fn forget_all(&self) {
        self.cache.lock().unwrap().clear();
    }

    fn byte_size(&self) -> usize {
        self.cache
            .lock()
            .unwrap()
            .values()
            .map(|e| match e {
                Some(Ok((b, _))) => b.len(),
                _ => 0,
            })
            .sum()
    }

    fn has_pending(&self) -> bool {
        self.cache.lock().unwrap().values().any(|e| e.is_none())
    }
}

/// Draws `emoji:<url>` pictures (see `gfm.rs`) at text height instead of
/// their full 64 px size.
pub struct EmojiLoader {
    textures: Mutex<HashMap<String, egui::TextureHandle>>,
}

/// Emoji height in points, about 1.25× body text like github.com.
const EMOJI_SIZE: f32 = 18.0;

impl EmojiLoader {
    pub fn install(ctx: &egui::Context) {
        ctx.add_texture_loader(Arc::new(EmojiLoader { textures: Default::default() }));
    }
}

impl egui::load::TextureLoader for EmojiLoader {
    fn id(&self) -> &str {
        egui::generate_loader_id!(EmojiLoader)
    }

    fn load(
        &self,
        ctx: &egui::Context,
        uri: &str,
        options: egui::TextureOptions,
        _size_hint: egui::load::SizeHint,
    ) -> egui::load::TextureLoadResult {
        use egui::load::{ImagePoll, SizedTexture, TexturePoll};
        let Some(url) = uri.strip_prefix(crate::gfm::EMOJI_SCHEME) else { return Err(LoadError::NotSupported) };
        let size = egui::vec2(EMOJI_SIZE, EMOJI_SIZE);
        if let Some(t) = self.textures.lock().unwrap().get(uri) {
            return Ok(TexturePoll::Ready { texture: SizedTexture::new(t.id(), size) });
        }
        // egui holds its texture-loader lock while calling us, so fetch and
        // decode through the image loaders, then make the texture ourselves.
        match ctx.try_load_image(url, egui::load::SizeHint::Scale(1.0.into()))? {
            ImagePoll::Pending { .. } => Ok(TexturePoll::Pending { size: Some(size) }),
            ImagePoll::Ready { image } => {
                let t = ctx.load_texture(uri, (*image).clone(), options);
                let id = t.id();
                self.textures.lock().unwrap().insert(uri.to_string(), t);
                Ok(TexturePoll::Ready { texture: SizedTexture::new(id, size) })
            }
        }
    }

    fn forget(&self, uri: &str) {
        self.textures.lock().unwrap().remove(uri);
    }

    fn forget_all(&self) {
        self.textures.lock().unwrap().clear();
    }

    fn end_pass(&self, _pass_index: u64) {}

    fn byte_size(&self) -> usize {
        self.textures.lock().unwrap().values().map(|t| t.byte_size()).sum()
    }
}
