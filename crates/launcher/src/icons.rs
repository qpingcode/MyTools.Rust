//! Display-only system icon loading. No scanning or application catalog lives here.
use crate::MainWindow;
use mytools_plugin_api::Icon;
use slint::{ComponentHandle, Image, Model, SharedPixelBuffer};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
    sync::mpsc,
    time::{Duration, Instant},
};

mod native;

const CACHE_LIMIT: usize = 1024;
/// System icons rarely change; refresh them a few times a day at most.
const CACHE_TTL: Duration = Duration::from_secs(6 * 60 * 60);
const FAILURE_RETRY: Duration = Duration::from_secs(60);

#[derive(Default)]
struct Cache {
    images: HashMap<PathBuf, (Instant, Image)>,
    pending: HashSet<PathBuf>,
}

pub struct Loader {
    requests: mpsc::SyncSender<PathBuf>,
    cache: Rc<RefCell<Cache>>,
    _updates: slint::JoinHandle<()>,
}

impl Loader {
    pub fn new(ui: &MainWindow) -> anyhow::Result<Self> {
        let (requests, work) = mpsc::sync_channel::<PathBuf>(128);
        let (completed, responses) = async_channel::unbounded();
        std::thread::spawn(move || {
            for path in work {
                let pixels = native::load(&path);
                if completed.send_blocking((path, pixels)).is_err() {
                    break;
                }
            }
        });
        let cache = Rc::new(RefCell::new(Cache::default()));
        let updated = cache.clone();
        let weak = ui.as_weak();
        let updates = slint::spawn_local(async move {
            while let Ok((path, pixels)) = responses.recv().await {
                let Some(ui) = weak.upgrade() else {
                    break;
                };
                let image = pixels
                    .map(|pixels| {
                        Image::from_rgba8(SharedPixelBuffer::clone_from_slice(
                            &pixels.rgba,
                            pixels.width,
                            pixels.height,
                        ))
                    })
                    .unwrap_or_default();
                {
                    let mut cache = updated.borrow_mut();
                    cache.pending.remove(&path);
                    if cache.images.len() >= CACHE_LIMIT && !cache.images.contains_key(&path) {
                        let oldest = cache
                            .images
                            .iter()
                            .min_by_key(|(_, (time, _))| time)
                            .map(|(key, _)| key.clone());
                        if let Some(oldest) = oldest {
                            cache.images.remove(&oldest);
                        }
                    }
                    // Failed extractions are retried after a minute instead of the full TTL.
                    let loaded = if image.size().width > 0 {
                        Instant::now()
                    } else {
                        Instant::now()
                            .checked_sub(CACHE_TTL - FAILURE_RETRY)
                            .unwrap_or_else(Instant::now)
                    };
                    cache.images.insert(path.clone(), (loaded, image.clone()));
                }
                let rows = ui.get_results();
                for index in 0..rows.row_count() {
                    let Some(mut row) = rows.row_data(index) else {
                        continue;
                    };
                    if row.native_path.as_str() == path.to_string_lossy() {
                        row.native_image = image.clone();
                        rows.set_row_data(index, row);
                    }
                }
            }
        })?;
        Ok(Self {
            requests,
            cache,
            _updates: updates,
        })
    }

    pub fn resolve(&self, icon: &Icon) -> (slint::SharedString, slint::SharedString, Image) {
        match icon {
            Icon::Name(name) => (name.as_str().into(), Default::default(), Image::default()),
            Icon::Native { native: path } => {
                let mut cache = self.cache.borrow_mut();
                let cached = cache
                    .images
                    .get(path)
                    .map(|(time, image)| (time.elapsed() < CACHE_TTL, image.clone()));
                // Expired icons stay visible while a fresh copy loads in the background.
                let image = match cached {
                    Some((true, image)) => image,
                    stale => {
                        if !cache.pending.contains(path)
                            && self.requests.try_send(path.clone()).is_ok()
                        {
                            cache.pending.insert(path.clone());
                        }
                        stale.map(|(_, image)| image).unwrap_or_default()
                    }
                };
                (
                    "builtin:app-window".into(),
                    path.to_string_lossy().as_ref().into(),
                    image,
                )
            }
        }
    }
}
