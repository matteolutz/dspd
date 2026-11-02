use std::{sync::Arc, time::Duration};

use gpui_kit::{App, AppContext, Entity, Task};

use crate::state::DspdHttpClient;

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct EngineStats {
    pub blocks: u64,
    pub frames: u64,
    /// Blocks that took longer to process than the audio they represented.
    pub overruns: u64,
    pub max_block_us: f64,
    /// Average DSP load in percent (processing time / audio time).
    pub avg_load_percent: f64,
}

impl EngineStats {
    pub(super) fn start_fetching(
        client: Arc<DspdHttpClient>,
        cx: &mut App,
    ) -> (Entity<Self>, Task<()>) {
        let stats_entity = cx.new(|_| Self::default());

        let task = cx.spawn({
            let stats_entity = stats_entity.clone();

            async move |cx| loop {
                let client = client.clone();

                let body: reqwest::Result<String> = cx
                    .background_spawn(async move {
                        // reqwest::blocking::get(stats_url).and_then(|res| res.text())
                        client.get("/stats").send().and_then(|res| res.text())
                    })
                    .await;

                if let Ok(body) = body {
                    if let Ok(stats) = serde_json::from_str::<EngineStats>(&body) {
                        cx.update_entity(&stats_entity, |this, cx| {
                            *this = stats;
                            cx.notify();
                        })
                    }
                }

                cx.background_executor().timer(Duration::from_secs(1)).await;
            }
        });

        (stats_entity, task)
    }
}
