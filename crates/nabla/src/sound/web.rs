//! The browser's audio output: an `AudioWorklet` (`assets/synth-worklet.js`) that runs the
//! synthesizer, built alone as `nabla_synth.wasm`, on the browser's audio rendering thread.
//!
//! The main thread only posts the processor each frame's controls. While it is busy (a slow
//! frame), the processor keeps rendering with the controls it has; when the messages stop for
//! a second (a hidden tab), it goes quiet on its own.
//!
//! Loading the script and the synthesizer takes a moment, so it happens in the background:
//! until it is done, frames' controls are dropped, and the processor starts with the next
//! ones.

use std::cell::RefCell;
use std::rc::Rc;

use js_sys::{Array, Object, Reflect};
use nabla_synth::{Controls, IonSound};
use wasm_bindgen::{JsCast as _, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{AudioContext, AudioWorkletNode, AudioWorkletNodeOptions, Response};

/// The processor's script and the synthesizer it runs, next to `index.html`.
const SCRIPT: &str = "synth-worklet.js";
const SYNTH: &str = "nabla_synth.wasm";

/// The name the script registers its processor under.
const PROCESSOR: &str = "nabla-synth";

/// The audio context, and the processor once loaded.
pub struct Output {
    context: AudioContext,
    node: Rc<RefCell<Option<AudioWorkletNode>>>,
    /// Why loading failed, until [`Self::take_error`] takes it.
    error: Rc<RefCell<Option<String>>>,
}

impl Output {
    /// Creates the audio context, and starts loading the processor.
    pub fn open() -> Result<Self, String> {
        let context = AudioContext::new().map_err(describe)?;
        let node: Rc<RefCell<Option<AudioWorkletNode>>> = Rc::default();
        let error: Rc<RefCell<Option<String>>> = Rc::default();
        wasm_bindgen_futures::spawn_local({
            let (context, node, error) = (context.clone(), Rc::clone(&node), Rc::clone(&error));
            async move {
                match load(&context).await {
                    Ok(loaded) => *node.borrow_mut() = Some(loaded),
                    Err(e) => *error.borrow_mut() = Some(e),
                }
            }
        });
        Ok(Self {
            context,
            node,
            error,
        })
    }

    /// Posts the processor the controls of this frame, once it is loaded.
    pub fn publish(&self, c: &Controls) {
        let Some(node) = &*self.node.borrow() else {
            return;
        };
        let ion = IonSound::ALL.iter().position(|&s| s == c.ion).unwrap_or(0);
        let message: Array = [
            c.thrust,
            c.torque,
            ion as f32,
            c.ion_gain,
            c.rcs_gain,
            c.master_gain,
        ]
        .into_iter()
        .map(JsValue::from)
        .collect();
        if let Err(e) = node.port().and_then(|port| port.post_message(&message)) {
            log::warn!("Audio output: {}", describe(e));
        }
    }

    /// Resumes the audio context, which the browser suspends until the page is interacted
    /// with.
    pub fn resume(&self) {
        if let Err(e) = self.context.resume() {
            log::warn!("Audio output: {}", describe(e));
        }
    }

    /// Why loading the processor failed, if it did.
    pub fn take_error(&self) -> Option<String> {
        self.error.borrow_mut().take()
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        // Closing the context stops the processor, and frees the audio device.
        if let Err(e) = self.context.close() {
            log::warn!("Audio output: {}", describe(e));
        }
    }
}

/// Fetches the synthesizer, adds the processor's script, and connects a processor to the
/// output.
async fn load(context: &AudioContext) -> Result<AudioWorkletNode, String> {
    let window = web_sys::window().ok_or("no window")?;
    let response: Response = JsFuture::from(window.fetch_with_str(SYNTH))
        .await
        .and_then(|response| response.dyn_into())
        .map_err(describe)?;
    if !response.ok() {
        return Err(format!("{SYNTH}: HTTP {}", response.status()));
    }
    let bytes = JsFuture::from(response.array_buffer().map_err(describe)?)
        .await
        .map_err(describe)?;
    let worklet = context.audio_worklet().map_err(describe)?;
    JsFuture::from(worklet.add_module(SCRIPT).map_err(describe)?)
        .await
        .map_err(describe)?;

    let processor_options = Object::new();
    Reflect::set(&processor_options, &"bytes".into(), &bytes).map_err(describe)?;
    let options = AudioWorkletNodeOptions::new();
    options.set_number_of_inputs(0);
    options.set_number_of_outputs(1);
    options.set_output_channel_count(&Array::of1(&2.into()));
    options.set_processor_options(Some(&processor_options));
    let node =
        AudioWorkletNode::new_with_options(context, PROCESSOR, &options).map_err(describe)?;
    node.connect_with_audio_node(&context.destination())
        .map_err(describe)?;
    Ok(node)
}

/// A JavaScript error's message.
fn describe(value: JsValue) -> String {
    match value.dyn_into::<js_sys::Error>() {
        Ok(error) => error.message().into(),
        Err(value) => value.as_string().unwrap_or_else(|| format!("{value:?}")),
    }
}
