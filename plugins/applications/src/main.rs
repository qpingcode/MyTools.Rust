use mytools_plugin_api::serve;
use std::{
    sync::{Arc, RwLock},
    time::Duration,
};

mod index;
mod packaged;
mod scan;

fn main() -> std::io::Result<()> {
    mytools_plugin_api::crash::install();
    let index = Arc::new(RwLock::new(index::Index::default()));
    let updated = index.clone();
    std::thread::spawn(move || {
        loop {
            // Build off-lock so slow directories never block the search protocol.
            let next = index::Index::new(scan::applications(&scan::roots()), packaged::apps());
            *updated.write().unwrap_or_else(|error| error.into_inner()) = next;
            std::thread::sleep(Duration::from_secs(30));
        }
    });
    serve(move |query| {
        index
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .search(query)
    })
}
