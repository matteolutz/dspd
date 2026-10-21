//! Editing a running rack from the control side.
//!
//! An [`Editor`] keeps a small model of what is on the audio thread (which
//! stage ids sit on which strip, their latencies, which meter slots they own)
//! and turns requests into [`EditBatch`]es:
//!
//! * the slow work (loading a plugin, allocating meter slots, building
//!   delay lines) happens here, on the control thread;
//! * the audio thread only swaps ready-made pieces in and out, between blocks;
//! * a request either succeeds completely (spec, model and audio thread all
//!   change) or fails with an [`EditError`] and changes nothing.
//!
//! Stage ids are stable: they are the position of the stage when the session
//! is built and are never reused afterwards, so a parameter change addressed
//! to a stage still reaches it after other stages were inserted or moved.

use std::sync::Arc;

use dspd_api::StageSpec;

use crate::edit::{EditBatch, EditOp, EditSender};
use crate::{
    DelayLine, MeterBank, MeterInfo, MeterWriter, Processor, Rack, Registry, SessionSpec, Stage,
    StageFactory, MAX_STAGES,
};

/// Why an edit was refused. Nothing has changed when you get one.
#[derive(Debug, thiserror::Error)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub enum EditError {
    #[error("no strip {0}")]
    NoStrip(usize),

    #[error("strip {strip} has no stage with id {id}")]
    NoStage { strip: usize, id: u32 },
    /// The position is past the end of the chain.
    #[error("position {index} is past the end of the chain ({len} stages)")]
    BadIndex { index: usize, len: usize },

    #[error("a strip holds at most {MAX_STAGES} stages")]
    TooManyStages,

    /// The stage spec names an unknown plugin, an unknown setting, or failed to load.
    #[error("{0}")]
    Invalid(String),

    /// `order` is not a permutation of the strip's stage ids.
    #[error("{0}")]
    BadOrder(String),

    /// The meter bank has no room for this stage's meters.
    #[error("no free meter slots")]
    NoMeterSlots,

    /// The audio thread has not caught up yet; retry.
    #[error("the audio thread has pending edits; retry")]
    QueueFull,
}

/// Outcome of [`Editor::insert`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Inserted {
    /// The new stage's id.
    pub id: u32,
    /// Where it sits in the chain.
    pub index: usize,
    /// Latency the stage reports.
    pub latency_frames: u32,
    /// Latency of the whole rack afterwards.
    pub rack_latency_frames: u32,
}

/// Hands out windows of consecutive meter slots from the bank's fixed room.
struct SlotAllocator {
    used: Vec<bool>,
}

impl SlotAllocator {
    fn new(capacity: usize) -> Self {
        Self {
            used: vec![false; capacity],
        }
    }

    fn mark(&mut self, base: usize, len: usize) {
        for u in &mut self.used[base..base + len] {
            *u = true;
        }
    }

    /// First window of `len` free slots.
    fn alloc(&mut self, len: usize) -> Option<usize> {
        let mut run = 0;
        for i in 0..self.used.len() {
            run = if self.used[i] { 0 } else { run + 1 };
            if run == len {
                let base = i + 1 - len;
                self.mark(base, len);
                return Some(base);
            }
        }
        None
    }

    fn free(&mut self, base: usize, len: usize) {
        for u in &mut self.used[base..base + len] {
            *u = false;
        }
    }
}

struct StageShadow {
    id: u32,
    latency: u32,
    /// First meter slot and how many.
    meters: Option<(usize, usize)>,
}

struct StripShadow {
    width: usize,
    stages: Vec<StageShadow>,
    next_id: u32,
    compensation: u32,
}

impl StripShadow {
    fn chain_latency(&self) -> u32 {
        self.stages.iter().map(|s| s.latency).sum()
    }

    fn position(&self, id: u32) -> Option<usize> {
        self.stages.iter().position(|s| s.id == id)
    }
}

/// Control-thread handle for editing the rack that an [`crate::Engine`] runs.
pub struct Editor {
    registry: Arc<Registry>,
    factory: Arc<dyn StageFactory>,
    tx: EditSender,
    meters: Arc<MeterBank>,
    slots: SlotAllocator,
    meter_infos: Vec<MeterInfo>,
    strips: Vec<StripShadow>,
    rack_latency: u32,
}

impl Editor {
    /// Model the rack as it is now. Call this before the rack moves into the
    /// engine; `spec` must be the session the rack was built from.
    pub fn new(
        rack: &Rack,
        registry: Arc<Registry>,
        factory: Arc<dyn StageFactory>,
        tx: EditSender,
    ) -> Self {
        let meters = Arc::clone(rack.meters());
        let meter_infos: Vec<MeterInfo> = rack.meter_infos().to_vec();
        let mut slots = SlotAllocator::new(meters.len());

        let strips = rack
            .strips()
            .iter()
            .enumerate()
            .map(|(si, strip)| {
                let chain = strip.chain();
                let stages = chain
                    .stage_order()
                    .iter()
                    .zip(chain.stages())
                    .map(|(&id, stage)| {
                        let mine: Vec<&MeterInfo> = meter_infos
                            .iter()
                            .filter(|i| i.strip == si && i.stage == id)
                            .collect();
                        let window = mine.first().map(|first| (first.slot, mine.len()));
                        if let Some((base, len)) = window {
                            slots.mark(base, len);
                        }
                        StageShadow {
                            id,
                            latency: stage.processor.latency_frames(),
                            meters: window,
                        }
                    })
                    .collect::<Vec<_>>();
                StripShadow {
                    width: strip.width(),
                    next_id: stages.len() as u32,
                    stages,
                    compensation: strip.compensation_frames() as u32,
                }
            })
            .collect();

        Self {
            registry,
            factory,
            tx,
            meters,
            slots,
            meter_infos,
            strips,
            rack_latency: rack.latency_frames(),
        }
    }

    // ---- read side -------------------------------------------------------

    pub fn strip_count(&self) -> usize {
        self.strips.len()
    }

    /// Stage ids of a strip in chain order.
    pub fn stage_ids(&self, strip: usize) -> Option<Vec<u32>> {
        self.strips
            .get(strip)
            .map(|s| s.stages.iter().map(|st| st.id).collect())
    }

    /// Position of a stage in its chain. Parameter writes into the session spec use this.
    pub fn index_of(&self, strip: usize, id: u32) -> Option<usize> {
        self.strips.get(strip)?.position(id)
    }

    /// Latency of the whole rack in frames.
    pub fn rack_latency_frames(&self) -> u32 {
        self.rack_latency
    }

    /// Every live meter, with the stage id in `stage`. Sorted by slot.
    pub fn meter_infos(&self) -> Arc<[MeterInfo]> {
        let mut v = self.meter_infos.clone();
        v.sort_by_key(|i| i.slot);
        v.into()
    }

    /// Free removed plugins now instead of with the next edit.
    pub fn reap(&mut self) {
        self.tx.reap();
    }

    // ---- edits -----------------------------------------------------------

    /// Add a stage to `strip` at `index` (the end if `None`).
    ///
    /// The plugin is loaded here, before anything touches the audio thread.
    /// `spec` is updated on success only.
    pub fn insert(
        &mut self,
        spec: &mut SessionSpec,
        strip: usize,
        index: Option<usize>,
        stage: StageSpec,
    ) -> Result<Inserted, EditError> {
        let shadow = self.strips.get(strip).ok_or(EditError::NoStrip(strip))?;
        if shadow.stages.len() >= MAX_STAGES {
            return Err(EditError::TooManyStages);
        }

        let len = shadow.stages.len();
        let at = index.unwrap_or(len); // insert at the end if None
        if at > len {
            return Err(EditError::BadIndex { index: at, len });
        }

        // Build against a copy of the spec that already contains the stage, so
        // every lookup (plugin type, settings, resources) sees it.
        let mut candidate = spec.clone();
        candidate.strips[strip].chain.insert(at, stage);
        candidate
            .check_plugins(&self.registry)
            .map_err(EditError::Invalid)?;

        let mut processor = candidate
            .build_stage(&self.registry, strip, at, self.factory.as_ref())
            .map_err(|e| EditError::Invalid(e.to_string()))?;

        let latency = processor.latency_frames();
        let meter_specs = processor.meter_specs();

        let window = if meter_specs.is_empty() {
            None
        } else {
            let base = self
                .slots
                .alloc(meter_specs.len())
                .ok_or(EditError::NoMeterSlots)?;

            for k in 0..meter_specs.len() {
                self.meters.set(base + k, 0.0);
            }

            processor.attach_meters(MeterWriter::new(
                Arc::clone(&self.meters),
                base,
                meter_specs.len(),
            ));

            Some((base, meter_specs.len()))
        };

        let id = self.strips[strip].next_id;
        let label = candidate.strips[strip].chain[at].label();
        let mut latencies = self.chain_latencies();
        latencies[strip] += latency;

        let mut ops = vec![EditOp::Insert {
            strip: strip as u32,
            index: at as u32,
            id,
            stage: Stage { label, processor },
        }];
        let (rack_latency, compensations) = self.plan_alignment(&latencies, &mut ops);

        if self.tx.send(EditBatch::new(ops)).is_err() {
            if let Some((base, len)) = window {
                self.slots.free(base, len);
            }
            return Err(EditError::QueueFull);
        }

        // Committed: the audio thread will apply the batch before its next block.
        *spec = candidate;
        let sh = &mut self.strips[strip];
        sh.next_id += 1;
        sh.stages.insert(
            at,
            StageShadow {
                id,
                latency,
                meters: window,
            },
        );
        if let Some((base, _)) = window {
            for (k, m) in meter_specs.into_iter().enumerate() {
                self.meter_infos.push(MeterInfo {
                    slot: base + k,
                    strip,
                    stage: id,
                    param_id: m.param_id,
                    name: m.name,
                });
            }
        }
        self.commit_alignment(rack_latency, compensations);
        Ok(Inserted {
            id,
            index: at,
            latency_frames: latency,
            rack_latency_frames: rack_latency,
        })
    }

    /// Remove the stage with this id. The plugin is freed on the control side
    /// once the audio thread has let go of it.
    pub fn remove(
        &mut self,
        spec: &mut SessionSpec,
        strip: usize,
        id: u32,
    ) -> Result<(), EditError> {
        let shadow = self.strips.get(strip).ok_or(EditError::NoStrip(strip))?;
        let pos = shadow
            .position(id)
            .ok_or(EditError::NoStage { strip, id })?;

        let mut latencies = self.chain_latencies();
        latencies[strip] -= shadow.stages[pos].latency;
        let mut ops = vec![EditOp::Remove {
            strip: strip as u32,
            id,
        }];
        let (rack_latency, compensations) = self.plan_alignment(&latencies, &mut ops);

        self.tx
            .send(EditBatch::new(ops))
            .map_err(|_| EditError::QueueFull)?;

        spec.strips[strip].chain.remove(pos);
        let removed = self.strips[strip].stages.remove(pos);
        if let Some((base, len)) = removed.meters {
            self.slots.free(base, len);
        }
        self.meter_infos
            .retain(|i| !(i.strip == strip && i.stage == id));
        self.commit_alignment(rack_latency, compensations);
        Ok(())
    }

    /// Move one stage so that it ends up at position `to`.
    pub fn move_stage(
        &mut self,
        spec: &mut SessionSpec,
        strip: usize,
        id: u32,
        to: usize,
    ) -> Result<(), EditError> {
        let shadow = self.strips.get(strip).ok_or(EditError::NoStrip(strip))?;
        let from = shadow
            .position(id)
            .ok_or(EditError::NoStage { strip, id })?;
        let len = shadow.stages.len();
        if to >= len {
            return Err(EditError::BadIndex { index: to, len });
        }
        let mut order: Vec<u32> = shadow.stages.iter().map(|s| s.id).collect();
        let moved = order.remove(from);
        order.insert(to, moved);
        self.reorder(spec, strip, &order)
    }

    /// Put the strip's stages in exactly this order. `order` must contain every
    /// stage id of the strip once.
    pub fn reorder(
        &mut self,
        spec: &mut SessionSpec,
        strip: usize,
        order: &[u32],
    ) -> Result<(), EditError> {
        let shadow = self.strips.get(strip).ok_or(EditError::NoStrip(strip))?;
        let current: Vec<u32> = shadow.stages.iter().map(|s| s.id).collect();

        let mut sorted_new = order.to_vec();
        sorted_new.sort_unstable();
        let mut sorted_cur = current.clone();
        sorted_cur.sort_unstable();
        if sorted_new != sorted_cur {
            return Err(EditError::BadOrder(format!(
                "order must list each of the strip's {} stage ids exactly once (current: {current:?})",
                current.len()
            )));
        }

        // Minimal-ish plan: walk the target order and move whatever is out of place.
        let mut sim = current.clone();
        let mut ops = Vec::new();
        for (k, &id) in order.iter().enumerate() {
            let at = sim.iter().position(|&x| x == id).expect("validated above");
            if at != k {
                sim.remove(at);
                sim.insert(k, id);
                ops.push(EditOp::Move {
                    strip: strip as u32,
                    id,
                    to: k as u32,
                });
            }
        }
        if ops.is_empty() {
            return Ok(());
        }
        self.tx
            .send(EditBatch::new(ops))
            .map_err(|_| EditError::QueueFull)?;

        let old_specs = std::mem::take(&mut spec.strips[strip].chain);
        let mut by_id: Vec<Option<StageSpec>> = old_specs.into_iter().map(Some).collect();
        spec.strips[strip].chain = order
            .iter()
            .map(|id| {
                by_id[current.iter().position(|c| c == id).unwrap()]
                    .take()
                    .expect("each id once")
            })
            .collect();
        let sh = &mut self.strips[strip];
        let mut old: Vec<Option<StageShadow>> = std::mem::take(&mut sh.stages)
            .into_iter()
            .map(Some)
            .collect();
        sh.stages = order
            .iter()
            .map(|id| {
                old[current.iter().position(|c| c == id).unwrap()]
                    .take()
                    .unwrap()
            })
            .collect();
        Ok(())
    }

    // ---- alignment -------------------------------------------------------

    fn chain_latencies(&self) -> Vec<u32> {
        self.strips.iter().map(StripShadow::chain_latency).collect()
    }

    /// Work out the delay each strip needs so all strips line up, and add the
    /// ops that change only what differs from now.
    fn plan_alignment(&self, latencies: &[u32], ops: &mut Vec<EditOp>) -> (u32, Vec<(usize, u32)>) {
        let target = latencies.iter().copied().max().unwrap_or(0);
        let mut changes = Vec::new();
        for (i, (sh, &lat)) in self.strips.iter().zip(latencies).enumerate() {
            let extra = target - lat;
            if extra != sh.compensation {
                ops.push(EditOp::SetCompensation {
                    strip: i as u32,
                    delay: DelayLine::new(sh.width, extra as usize),
                });
                changes.push((i, extra));
            }
        }
        if target != self.rack_latency {
            ops.push(EditOp::SetLatency { frames: target });
        }
        (target, changes)
    }

    fn commit_alignment(&mut self, rack_latency: u32, compensations: Vec<(usize, u32)>) {
        self.rack_latency = rack_latency;
        for (i, extra) in compensations {
            self.strips[i].compensation = extra;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{edit_channel, param_channel, AudioBlock, Engine, ParamEvent, Processor};
    use dspd_api::{BoxedProcessor, ClapSpec, ParamDef, PluginContext, PluginDescriptor};
    use std::sync::atomic::{AtomicUsize, Ordering};

    // ---- small test plugins -------------------------------------------

    /// output = input + c; does not commute with gain, so order is audible.
    struct Add(f32);
    impl Processor for Add {
        fn name(&self) -> &str {
            "add"
        }
        fn process(&mut self, input: &AudioBlock, output: &mut AudioBlock) {
            output.copy_from(input);
            for c in 0..output.num_channels() {
                for s in output.channel_mut(c) {
                    *s += self.0;
                }
            }
        }
        fn set_param(&mut self, id: u32, value: f64) {
            if id == 0 {
                self.0 = value as f32;
            }
        }
    }
    static ADD: PluginDescriptor = PluginDescriptor {
        id: "add",
        name: "Add",
        description: "adds a constant",
        params: &[ParamDef {
            id: 0,
            key: "c",
            name: "c",
            min: -10.0,
            max: 10.0,
            default: 0.0,
        }],
        create: |_| Box::new(Add(0.0)),
    };

    /// Delays by `frames` and reports it as latency.
    struct Delay {
        frames: usize,
        ring: Vec<Vec<f32>>,
        pos: usize,
    }
    impl Processor for Delay {
        fn name(&self) -> &str {
            "delay"
        }
        fn process(&mut self, input: &AudioBlock, output: &mut AudioBlock) {
            output.copy_from(input);
            if self.frames == 0 {
                return;
            }
            for (c, line) in self.ring.iter_mut().enumerate() {
                let mut p = self.pos;
                for s in output.channel_mut(c) {
                    std::mem::swap(s, &mut line[p]);
                    p = (p + 1) % self.frames;
                }
            }
            self.pos = (self.pos + input.frames()) % self.frames;
        }
        fn latency_frames(&self) -> u32 {
            self.frames as u32
        }
        fn set_param(&mut self, id: u32, value: f64) {
            if id == 0 {
                self.frames = value as usize;
                for line in &mut self.ring {
                    *line = vec![0.0; self.frames.max(1)];
                }
            }
        }
    }
    static DELAY: PluginDescriptor = PluginDescriptor {
        id: "delay",
        name: "Delay",
        description: "delays and reports latency",
        params: &[ParamDef {
            id: 0,
            key: "frames",
            name: "frames",
            min: 0.0,
            max: 4800.0,
            default: 0.0,
        }],
        create: |ctx| {
            Box::new(Delay {
                frames: 0,
                ring: vec![vec![0.0; 1]; ctx.channels],
                pos: 0,
            })
        },
    };

    /// Counts how many instances were dropped.
    static DROPS: AtomicUsize = AtomicUsize::new(0);
    struct Tracked;
    impl Processor for Tracked {
        fn name(&self) -> &str {
            "tracked"
        }
        fn process(&mut self, input: &AudioBlock, output: &mut AudioBlock) {
            output.copy_from(input);
        }
    }
    impl Drop for Tracked {
        fn drop(&mut self) {
            DROPS.fetch_add(1, Ordering::SeqCst);
        }
    }
    static TRACKED: PluginDescriptor = PluginDescriptor {
        id: "tracked",
        name: "Tracked",
        description: "counts drops",
        params: &[],
        create: |_: &PluginContext| Box::new(Tracked),
    };

    struct NoClap;
    impl StageFactory for NoClap {
        fn load_clap(&self, _: &ClapSpec, _: usize) -> Result<BoxedProcessor, String> {
            Err("no clap in tests".into())
        }
    }

    fn registry() -> Arc<Registry> {
        Arc::new(
            Registry::from_descriptors(&[
                &dspd_plugin_gain::DESCRIPTOR,
                &dspd_plugin_peak_meter::DESCRIPTOR,
                &ADD,
                &DELAY,
                &TRACKED,
            ])
            .unwrap(),
        )
    }

    struct Rig {
        engine: Engine,
        editor: Editor,
        spec: SessionSpec,
        params: crate::ParamSender,
    }

    fn rig_with(json: &str, queue: usize) -> Rig {
        let spec = SessionSpec::from_json_str(json).unwrap();
        let reg = registry();
        let rack = spec.build_rack(&reg, &NoClap).unwrap();
        let (tx, rx) = edit_channel(queue);
        let editor = Editor::new(&rack, reg, Arc::new(NoClap), tx);
        let (params, prx) = param_channel(64);
        let engine = Engine::new(rack, prx, spec.sample_rate).with_edits(rx);
        Rig {
            engine,
            editor,
            spec,
            params,
        }
    }

    fn rig(json: &str) -> Rig {
        rig_with(json, 8)
    }

    /// Run one block of a constant on channel 0 and return the last output sample.
    fn level(engine: &mut Engine, value: f32) -> f32 {
        let mut input = AudioBlock::new(1, 64);
        input.channel_mut(0).fill(value);
        let mut output = AudioBlock::new(1, 64);
        engine.process(&input, &mut output);
        output.channel(0)[63]
    }

    fn stage(json: &str) -> StageSpec {
        serde_json::from_str(json).unwrap()
    }

    const ONE_STRIP: &str = r#"{"max_block":64,"strips":[{"input":0,"chain":[]}]}"#;

    // ---- tests ---------------------------------------------------------

    #[test]
    fn insert_changes_the_audio_and_the_spec() {
        let mut r = rig(ONE_STRIP);
        assert!((level(&mut r.engine, 1.0) - 1.0).abs() < 1e-6);

        let ins = r
            .editor
            .insert(
                &mut r.spec,
                0,
                None,
                stage(r#"{"type":"gain","gain_db":-6.0206}"#),
            )
            .unwrap();
        assert_eq!((ins.id, ins.index, ins.latency_frames), (0, 0, 0));
        assert!(
            (level(&mut r.engine, 1.0) - 0.5).abs() < 1e-3,
            "the stage starts at its stored gain, without a ramp"
        );
        assert_eq!(r.spec.strips[0].chain.len(), 1);
        assert_eq!(r.editor.stage_ids(0).unwrap(), vec![0]);

        // insert at the front: new id, old stage keeps its id
        let ins = r
            .editor
            .insert(
                &mut r.spec,
                0,
                Some(0),
                stage(r#"{"type":"gain","gain_db":-6.0206}"#),
            )
            .unwrap();
        assert_eq!((ins.id, ins.index), (1, 0));
        assert_eq!(r.editor.stage_ids(0).unwrap(), vec![1, 0]);
        assert!((level(&mut r.engine, 1.0) - 0.25).abs() < 1e-3);
    }

    #[test]
    fn params_still_reach_the_right_stage_after_edits() {
        let mut r =
            rig(r#"{"max_block":64,"strips":[{"input":0,"chain":[{"type":"add","c":0.5}]}]}"#);
        // initial stage has id 0 == position
        r.editor
            .insert(
                &mut r.spec,
                0,
                Some(0),
                stage(r#"{"type":"gain","gain_db":-6.0206}"#),
            )
            .unwrap(); // id 1, in front
                       // the add stage is now second but still addressed as id 0
        r.params
            .send(ParamEvent {
                strip: 0,
                stage: 0,
                param_id: 0,
                value: 2.0,
            })
            .unwrap();
        // gain (0.5) then add 2.0
        assert!((level(&mut r.engine, 1.0) - 2.5).abs() < 1e-3);
        assert_eq!(r.editor.index_of(0, 0), Some(1));
        assert_eq!(r.editor.index_of(0, 1), Some(0));
        assert_eq!(r.editor.index_of(0, 9), None);
    }

    #[test]
    fn an_event_sent_right_after_an_insert_finds_the_new_stage() {
        // Both are queued before the audio thread runs again; edits are applied first.
        let mut r = rig(ONE_STRIP);
        let ins = r
            .editor
            .insert(&mut r.spec, 0, None, stage(r#"{"type":"add","c":0}"#))
            .unwrap();
        r.params
            .send(ParamEvent {
                strip: 0,
                stage: ins.id,
                param_id: 0,
                value: 3.0,
            })
            .unwrap();
        assert!((level(&mut r.engine, 1.0) - 4.0).abs() < 1e-6);
    }

    #[test]
    fn reorder_and_move_change_the_signal_path() {
        let mut r = rig(r#"{"max_block":64,"strips":[{"input":0,"chain":[
            {"type":"gain","gain_db":-6.0206},{"type":"add","c":0.5}]}]}"#);
        assert!((level(&mut r.engine, 1.0) - 1.0).abs() < 1e-3); // 1*0.5 + 0.5

        r.editor.move_stage(&mut r.spec, 0, 1, 0).unwrap(); // add first
        assert!((level(&mut r.engine, 1.0) - 0.75).abs() < 1e-3); // (1+0.5)*0.5
        assert_eq!(r.editor.stage_ids(0).unwrap(), vec![1, 0]);
        assert!(matches!(&r.spec.strips[0].chain[0], StageSpec::Native(n) if n.plugin == "add"));

        r.editor.reorder(&mut r.spec, 0, &[0, 1]).unwrap();
        assert!((level(&mut r.engine, 1.0) - 1.0).abs() < 1e-3);
        assert!(matches!(&r.spec.strips[0].chain[0], StageSpec::Native(n) if n.plugin == "gain"));

        // already in order: accepted, nothing queued
        r.editor.reorder(&mut r.spec, 0, &[0, 1]).unwrap();
    }

    #[test]
    fn bad_orders_and_moves_are_rejected_without_changes() {
        let mut r = rig(
            r#"{"max_block":64,"strips":[{"input":0,"chain":[{"type":"gain"},{"type":"add"}]}]}"#,
        );
        for bad in [&[0u32][..], &[0, 0], &[0, 1, 2], &[0, 5]] {
            assert!(
                matches!(
                    r.editor.reorder(&mut r.spec, 0, bad),
                    Err(EditError::BadOrder(_))
                ),
                "{bad:?}"
            );
        }
        assert_eq!(
            r.editor.move_stage(&mut r.spec, 0, 0, 2),
            Err(EditError::BadIndex { index: 2, len: 2 })
        );
        assert_eq!(
            r.editor.move_stage(&mut r.spec, 0, 9, 0),
            Err(EditError::NoStage { strip: 0, id: 9 })
        );
        assert_eq!(
            r.editor.reorder(&mut r.spec, 4, &[]),
            Err(EditError::NoStrip(4))
        );
        assert_eq!(r.editor.stage_ids(0).unwrap(), vec![0, 1]);
    }

    #[test]
    fn removed_plugins_are_freed_on_the_control_side_not_the_audio_thread() {
        let mut r = rig(ONE_STRIP);
        let before = DROPS.load(Ordering::SeqCst);
        let ins = r
            .editor
            .insert(&mut r.spec, 0, None, stage(r#"{"type":"tracked"}"#))
            .unwrap();
        level(&mut r.engine, 1.0);

        r.editor.remove(&mut r.spec, 0, ins.id).unwrap();
        level(&mut r.engine, 1.0); // the audio thread applies the removal
        assert_eq!(
            DROPS.load(Ordering::SeqCst),
            before,
            "the audio thread must not drop the plugin"
        );
        assert!(r.spec.strips[0].chain.is_empty());

        r.editor.reap();
        assert_eq!(
            DROPS.load(Ordering::SeqCst),
            before + 1,
            "freed by the control side"
        );
    }

    #[test]
    fn insert_remove_keep_strips_aligned_in_latency() {
        let mut r = rig(r#"{"max_block":64,"strips":[
            {"input":0,"output":0,"chain":[]},
            {"input":1,"output":1,"chain":[]}]}"#);
        assert_eq!(r.editor.rack_latency_frames(), 0);

        let ins = r
            .editor
            .insert(
                &mut r.spec,
                1,
                None,
                stage(r#"{"type":"delay","frames":100}"#),
            )
            .unwrap();
        assert_eq!((ins.latency_frames, ins.rack_latency_frames), (100, 100));
        let mut input = AudioBlock::new(2, 64);
        let mut output = AudioBlock::new(2, 64);
        r.engine.process(&input, &mut output);
        assert_eq!(r.engine.latency_frames(), 100);
        assert_eq!(
            r.engine.rack().strips()[0].compensation_frames(),
            100,
            "the other strip is delayed to match"
        );
        assert_eq!(r.engine.rack().strips()[1].compensation_frames(), 0);

        // an impulse on both inputs comes out of both outputs at the same frame
        let mut first = [None, None];
        for block in 0..6 {
            input.channel_mut(0).fill(0.0);
            input.channel_mut(1).fill(0.0);
            if block == 0 {
                input.channel_mut(0)[0] = 1.0;
                input.channel_mut(1)[0] = 1.0;
            }
            r.engine.process(&input, &mut output);
            for (c, slot) in first.iter_mut().enumerate() {
                if slot.is_none() {
                    *slot = output
                        .channel(c)
                        .iter()
                        .position(|s| *s != 0.0)
                        .map(|p| block * 64 + p);
                }
            }
        }
        assert_eq!(first[0], Some(100));
        assert_eq!(first[1], Some(100));

        r.editor.remove(&mut r.spec, 1, ins.id).unwrap();
        r.engine.process(&input, &mut output);
        assert_eq!(r.engine.latency_frames(), 0);
        assert_eq!(r.engine.rack().strips()[0].compensation_frames(), 0);
    }

    #[test]
    fn meters_follow_inserted_and_removed_stages() {
        let mut r = rig(ONE_STRIP);
        assert!(r.editor.meter_infos().is_empty());

        let a = r
            .editor
            .insert(&mut r.spec, 0, None, stage(r#"{"type":"peak_meter"}"#))
            .unwrap();
        let infos = r.editor.meter_infos();
        assert_eq!(
            (
                infos.len(),
                infos[0].slot,
                infos[0].strip,
                infos[0].stage,
                infos[0].name.as_str()
            ),
            (1, 0, 0, a.id, "Peak")
        );
        level(&mut r.engine, 0.5);
        let bank = r.engine.meters();
        assert!(
            (bank.get(0) - 20.0 * 0.5f32.log10()).abs() < 0.1,
            "the new stage writes into its slot: {}",
            bank.get(0)
        );

        let b = r
            .editor
            .insert(&mut r.spec, 0, None, stage(r#"{"type":"peak_meter"}"#))
            .unwrap();
        assert_eq!(r.editor.meter_infos()[1].slot, 1);
        assert_eq!(r.editor.meter_infos()[1].stage, b.id);

        r.editor.remove(&mut r.spec, 0, a.id).unwrap();
        let infos = r.editor.meter_infos();
        assert_eq!(
            (infos.len(), infos[0].stage),
            (1, b.id),
            "the removed stage's meter is gone"
        );

        // its slot is reused, and starts from zero
        let c = r
            .editor
            .insert(&mut r.spec, 0, None, stage(r#"{"type":"peak_meter"}"#))
            .unwrap();
        let infos = r.editor.meter_infos();
        assert_eq!((infos[0].slot, infos[0].stage), (0, c.id));
        assert_eq!(r.engine.meters().get(0), 0.0);
    }

    #[test]
    fn refused_edits_change_nothing() {
        let mut r = rig_with(ONE_STRIP, 1);
        let spec_before = r.spec.to_json_pretty().unwrap();

        let e = r
            .editor
            .insert(&mut r.spec, 0, None, stage(r#"{"type":"reverb"}"#))
            .unwrap_err();
        assert!(
            matches!(&e, EditError::Invalid(m) if m.contains("unknown plugin type `reverb`")),
            "{e}"
        );
        let e = r
            .editor
            .insert(
                &mut r.spec,
                0,
                None,
                stage(r#"{"type":"gain","gane_db":1}"#),
            )
            .unwrap_err();
        assert!(
            matches!(&e, EditError::Invalid(m) if m.contains("no setting `gane_db`")),
            "{e}"
        );
        let e = r
            .editor
            .insert(
                &mut r.spec,
                0,
                None,
                stage(r#"{"type":"clap","path":"x.clap"}"#),
            )
            .unwrap_err();
        assert!(
            matches!(&e, EditError::Invalid(m) if m.contains("no clap in tests")),
            "{e}"
        );
        assert_eq!(
            r.editor
                .insert(&mut r.spec, 3, None, stage(r#"{"type":"gain"}"#)),
            Err(EditError::NoStrip(3))
        );
        assert_eq!(
            r.editor
                .insert(&mut r.spec, 0, Some(5), stage(r#"{"type":"gain"}"#)),
            Err(EditError::BadIndex { index: 5, len: 0 })
        );
        assert_eq!(
            r.editor.remove(&mut r.spec, 0, 0),
            Err(EditError::NoStage { strip: 0, id: 0 })
        );
        assert_eq!(r.spec.to_json_pretty().unwrap(), spec_before);
        assert!(r.editor.stage_ids(0).unwrap().is_empty());

        // a full queue is a retryable error that leaves everything as it was, including meter slots
        r.editor
            .insert(&mut r.spec, 0, None, stage(r#"{"type":"gain"}"#))
            .unwrap();
        let e = r
            .editor
            .insert(&mut r.spec, 0, None, stage(r#"{"type":"peak_meter"}"#))
            .unwrap_err();
        assert_eq!(e, EditError::QueueFull);
        assert_eq!(r.spec.strips[0].chain.len(), 1);
        assert!(r.editor.meter_infos().is_empty());
        level(&mut r.engine, 1.0); // audio thread drains the queue
        let ok = r
            .editor
            .insert(&mut r.spec, 0, None, stage(r#"{"type":"peak_meter"}"#))
            .unwrap();
        assert_eq!(
            r.editor.meter_infos()[0].slot,
            0,
            "the slot reserved by the failed attempt was given back"
        );
        assert_eq!(ok.id, 1, "ids are not consumed by failed attempts");
    }

    #[test]
    fn a_chain_cannot_outgrow_its_reserved_room() {
        let mut r = rig_with(ONE_STRIP, 128);
        for _ in 0..MAX_STAGES {
            r.editor
                .insert(&mut r.spec, 0, None, stage(r#"{"type":"gain"}"#))
                .unwrap();
            level(&mut r.engine, 1.0);
        }
        assert_eq!(
            r.editor
                .insert(&mut r.spec, 0, None, stage(r#"{"type":"gain"}"#)),
            Err(EditError::TooManyStages)
        );
    }

    #[test]
    fn the_edited_spec_round_trips_through_json() {
        let mut r = rig(ONE_STRIP);
        r.editor
            .insert(
                &mut r.spec,
                0,
                None,
                stage(r#"{"type":"gain","gain_db":-3}"#),
            )
            .unwrap();
        r.editor
            .insert(&mut r.spec, 0, Some(0), stage(r#"{"type":"add","c":1}"#))
            .unwrap();
        let again = SessionSpec::from_json_str(&r.spec.to_json_pretty().unwrap()).unwrap();
        let rebuilt = again.build_rack(&registry(), &NoClap).unwrap();
        let labels: Vec<_> = rebuilt.strips()[0]
            .chain()
            .stages()
            .iter()
            .map(|s| s.label.as_str())
            .collect();
        assert_eq!(labels, ["add", "gain"]);
    }

    #[test]
    fn slot_allocator_finds_gaps_and_reuses_freed_windows() {
        let mut a = SlotAllocator::new(6);
        assert_eq!(a.alloc(2), Some(0));
        assert_eq!(a.alloc(3), Some(2));
        assert_eq!(a.alloc(2), None, "only one slot left");
        a.free(0, 2);
        assert_eq!(a.alloc(2), Some(0));
        a.free(2, 3);
        assert_eq!(a.alloc(1), Some(2));
        assert_eq!(a.alloc(4), None);
    }
}
