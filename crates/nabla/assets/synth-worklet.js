// The thruster sounds' AudioWorkletProcessor: plays nabla_synth.wasm (the `nabla-synth`
// crate, built alone) on the browser's audio rendering thread, so that slow frames on the
// page's main thread cannot starve the audio. The app posts the controls once per frame, as
// [thrust, torque, ion, ion_gain, rcs_gain, master_gain]; between messages the synthesizer
// keeps the last ones. See src/sound/web.rs and docs/audio_worklet.typ.

const QUANTUM = 128; // the most frames nabla_synth renders at once

class NablaSynth extends AudioWorkletProcessor {
  constructor({ processorOptions }) {
    super();
    this.port.onmessage = ({ data }) => {
      // Until the synthesizer is ready, only the latest controls matter.
      this.controls = data;
      this.synth?.synth_set(...data);
    };
    WebAssembly.instantiate(processorOptions.bytes).then(({ instance }) => {
      const synth = instance.exports;
      const out = synth.synth_init(sampleRate);
      // nabla_synth allocates nothing after synth_init, so memory never grows and these
      // views stay valid.
      this.left = new Float32Array(synth.memory.buffer, out, QUANTUM);
      this.right = new Float32Array(synth.memory.buffer, out + 4 * QUANTUM, QUANTUM);
      if (this.controls) synth.synth_set(...this.controls);
      this.synth = synth;
    });
  }

  process(inputs, outputs) {
    if (!this.synth) return true; // still loading: the outputs stay silent
    const [left, right] = outputs[0];
    const frames = Math.min(left.length, QUANTUM);
    this.synth.synth_render(frames);
    left.set(this.left.subarray(0, frames));
    right.set(this.right.subarray(0, frames));
    return true;
  }
}

registerProcessor("nabla-synth", NablaSynth);
