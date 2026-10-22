use dspd_api::{ParamDef, PluginDescriptor};

pub const BAND_COUNT: usize = 6;
pub const PARAMS_PER_BAND: u32 = 8;

use crate::DynamicEq;

macro_rules! all_band_params {
    ($($band:literal),* $(,)?) => {
        &[
            $(
                ParamDef {
                    id: (($band - 1) * PARAMS_PER_BAND),
                    key: concat!("freq_hz_", stringify!($band)),
                    name: concat!("Band ", stringify!($band), " Frequency (Hz)").to_string(),
                    min: 20.0,
                    max: 20_000.0,
                    default: 1000.0,
                },
                ParamDef {
                    id: (($band - 1) * PARAMS_PER_BAND) + 1,
                    key: concat!("q_", stringify!($band)),
                    name: concat!("Band ", stringify!($band), " Q").to_string(),
                    min: 0.1,
                    max: 18.0,
                    default: 1.0,
                },
                ParamDef {
                    id: (($band - 1) * PARAMS_PER_BAND) + 2,
                    key: concat!("static_gain_db_", stringify!($band)),
                    name: concat!("Band ", stringify!($band), " Static Gain (dB)").to_string(),
                    min: -24.0,
                    max: 24.0,
                    default: 0.0,
                },
                ParamDef {
                    id: (($band - 1) * PARAMS_PER_BAND) + 3,
                    key: concat!("threshold_db_", stringify!($band)),
                    name: concat!("Band ", stringify!($band), " Threshold (dB)").to_string(),
                    min: -90.0,
                    max: 0.0,
                    default: 0.0,
                },
                ParamDef {
                    id: (($band - 1) * PARAMS_PER_BAND) + 4,
                    key: concat!("ratio_", stringify!($band)),
                    name: concat!("Band ", stringify!($band), " Ratio").to_string(),
                    min: 1.0,
                    max: 20.0,
                    default: 1.0,
                },
                ParamDef {
                    id: (($band - 1) * PARAMS_PER_BAND) + 5,
                    key: concat!("range_db_", stringify!($band)),
                    name: concat!("Band ", stringify!($band), " Range (dB)").to_string(),
                    min: -24.0,
                    max: 24.0,
                    default: 0.0,
                },
                ParamDef {
                    id: (($band - 1) * PARAMS_PER_BAND) + 6,
                    key: concat!("attack_ms_", stringify!($band)),
                    name: concat!("Band ", stringify!($band), " Attack (ms)").to_string(),
                    min: 0.1,
                    max: 500.0,
                    default: 30.0,
                },
                ParamDef {
                    id: (($band - 1) * PARAMS_PER_BAND) + 7,
                    key: concat!("release_ms_", stringify!($band)),
                    name: concat!("Band ", stringify!($band), " Release (ms)").to_string(),
                    min: 0.1,
                    max: 2000.0,
                    default: 100.0,
                },
            )*
        ]
    };
}

pub fn descriptor() -> PluginDescriptor {
    PluginDescriptor {
        id: "dyn_eq",
        name: "Dynamic EQ".to_string(),
        description: "6 Band Dynamic EQ".to_string(),
        params: all_band_params!(1, 2, 3, 4, 5, 6).to_vec(),
        create: Box::new(|ctx| Box::new(DynamicEq::new(ctx.sample_rate as f32, ctx.channels))),
    }
}
