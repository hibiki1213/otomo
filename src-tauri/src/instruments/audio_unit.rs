//! AudioUnit instrument hosting (macOS).
//!
//! Plugins are instantiated with `kAudioComponentInstantiation_LoadOutOfProcess`:
//! macOS runs them in its AUHostingService process (AUv3 extensions run in
//! their own extension process anyway), so a crashing plugin cannot take the
//! app down — the same isolation Logic Pro uses. Rendering goes through the
//! cached `renderBlock` and MIDI through `scheduleMIDIEventBlock`, both
//! realtime-safe. The editor is the plugin's own view controller (or Apple's
//! generic parameter view) in a window owned by the app's main thread.

use std::ffi::c_void;
use std::ptr::{self, NonNull};
use std::sync::mpsc;
use std::time::Duration;

use block2::{DynBlock, RcBlock};
use objc2::rc::Retained;
use objc2::{msg_send, AnyThread, MainThreadMarker};
use objc2_app_kit::{NSViewController, NSWindow, NSWindowStyleMask};
use objc2_audio_toolbox::{
    AUAudioUnit, AUEventSampleTimeImmediate, AudioComponent, AudioComponentCopyName, AudioComponentDescription,
    AudioComponentFindNext, AudioComponentFlags, AudioComponentGetDescription, AudioComponentInstantiationOptions,
    AudioUnitRenderActionFlags, kAudioUnitType_MusicDevice,
};
use objc2_avf_audio::AVAudioFormat;
use objc2_core_audio_kit::{AUAudioUnit_ViewController, AUGenericViewController};
use objc2_core_audio_types::{AudioBuffer, AudioBufferList, AudioTimeStamp, AudioTimeStampFlags};
use objc2_core_foundation::{CFRetained, CFString};
use objc2_foundation::{NSError, NSString};
use serde::Serialize;

use crate::engine::player::Instrument;

const MAX_FRAMES: u32 = 1024;

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    /// "type-subtype-manufacturer" as hex four-char codes.
    pub id: String,
    pub name: String,
    pub manufacturer: String,
    pub format: &'static str,
}

fn fourcc_hex(d: &AudioComponentDescription) -> String {
    format!("{:08x}-{:08x}-{:08x}", d.componentType, d.componentSubType, d.componentManufacturer)
}

fn parse_id(id: &str) -> Result<AudioComponentDescription, String> {
    let parts: Vec<u32> = id
        .split('-')
        .map(|p| u32::from_str_radix(p, 16).map_err(|e| e.to_string()))
        .collect::<Result<_, _>>()?;
    let [ty, sub, manu] = parts[..] else { return Err(format!("bad plugin id {id}")) };
    Ok(AudioComponentDescription {
        componentType: ty,
        componentSubType: sub,
        componentManufacturer: manu,
        componentFlags: 0,
        componentFlagsMask: 0,
    })
}

/// Installed AudioUnit instruments (AUv2 and AUv3).
pub fn list_instruments() -> Vec<PluginInfo> {
    let mut query = AudioComponentDescription {
        componentType: kAudioUnitType_MusicDevice,
        componentSubType: 0,
        componentManufacturer: 0,
        componentFlags: 0,
        componentFlagsMask: 0,
    };
    let mut out = Vec::new();
    let mut comp: AudioComponent = ptr::null_mut();
    loop {
        // SAFETY: plain C enumeration over registered components.
        comp = unsafe { AudioComponentFindNext(comp, NonNull::from(&mut query)) };
        if comp.is_null() {
            break;
        }
        let mut desc = query;
        let mut name_ref: *const CFString = ptr::null();
        unsafe {
            AudioComponentGetDescription(comp, NonNull::from(&mut desc));
            AudioComponentCopyName(comp, NonNull::from(&mut name_ref));
        }
        let full = NonNull::new(name_ref as *mut CFString)
            // CopyName follows the Create rule: take ownership so it's released.
            .map(|p| unsafe { CFRetained::from_raw(p) }.to_string())
            .unwrap_or_default();
        // By convention "Manufacturer: Name".
        let (manufacturer, name) = match full.split_once(": ") {
            Some((m, n)) => (m.to_string(), n.to_string()),
            None => (String::new(), full.clone()),
        };
        let v3 = AudioComponentFlags(desc.componentFlags).contains(AudioComponentFlags::IsV3AudioUnit);
        out.push(PluginInfo { id: fourcc_hex(&desc), name, manufacturer, format: if v3 { "AUv3" } else { "AU" } });
    }
    out.sort_by(|a, b| (&a.manufacturer, &a.name).cmp(&(&b.manufacturer, &b.name)));
    out
}

/// A retained AUAudioUnit that may cross threads. AUAudioUnit's render and
/// MIDI blocks are designed to be called from the render thread while the
/// object itself is configured from others.
#[derive(Clone)]
pub struct SharedAudioUnit(Retained<AUAudioUnit>);
unsafe impl Send for SharedAudioUnit {}
unsafe impl Sync for SharedAudioUnit {}

type RenderBlock = RcBlock<
    dyn Fn(
        NonNull<AudioUnitRenderActionFlags>,
        NonNull<AudioTimeStamp>,
        u32,
        isize,
        NonNull<AudioBufferList>,
        *mut DynBlock<
            dyn Fn(NonNull<AudioUnitRenderActionFlags>, NonNull<AudioTimeStamp>, u32, isize, NonNull<AudioBufferList>) -> i32,
        >,
    ) -> i32,
>;
type MidiBlock = RcBlock<dyn Fn(i64, u8, isize, NonNull<u8>)>;

/// AudioBufferList with room for two (non-interleaved) buffers.
#[repr(C)]
struct StereoBufferList {
    number_buffers: u32,
    buffers: [AudioBuffer; 2],
}

pub struct AudioUnitInstrument {
    unit: SharedAudioUnit,
    render_block: RenderBlock,
    midi_block: MidiBlock,
    /// MIDI channel used for this instrument's messages (9 = GM drums).
    channel: u8,
    sample_time: f64,
    held: [bool; 128],
    pub out_of_process: bool,
}

unsafe impl Send for AudioUnitInstrument {}

impl AudioUnitInstrument {
    /// Instantiate (out of process), configure and allocate render resources.
    /// Blocking; must not run on the main thread.
    pub fn load(id: &str, sample_rate: u32, channel: u8) -> Result<Self, String> {
        let desc = parse_id(id)?;
        let unit = instantiate(desc, AudioComponentInstantiationOptions::LoadOutOfProcess)?;

        unsafe {
            let format = AVAudioFormat::initStandardFormatWithSampleRate_channels(
                AVAudioFormat::alloc(),
                f64::from(sample_rate),
                2,
            )
            .ok_or("could not create audio format")?;
            let busses = unit.outputBusses();
            if busses.count() == 0 {
                return Err("plugin has no audio output".into());
            }
            let bus = busses.objectAtIndexedSubscript(0);
            let mut error: *mut NSError = ptr::null_mut();
            let ok: bool = msg_send![&*bus, setFormat: &*format, error: &mut error];
            if !ok {
                return Err(format!("plugin rejected {sample_rate} Hz stereo output"));
            }
            unit.setMaximumFramesToRender(MAX_FRAMES);

            // Fetch and keep the blocks before allocating render resources.
            let render_ptr = unit.renderBlock();
            let midi_ptr = unit.scheduleMIDIEventBlock();
            let render_block = RcBlock::copy(render_ptr).ok_or("plugin has no render block")?;
            let midi_block = RcBlock::copy(midi_ptr).ok_or("plugin does not accept MIDI")?;

            unit.allocateRenderResourcesAndReturnError().map_err(|e| e.localizedDescription().to_string())?;
            let out_of_process = !unit.isLoadedInProcess();
            Ok(Self {
                unit: SharedAudioUnit(unit),
                render_block,
                midi_block,
                channel: channel & 15,
                sample_time: 0.0,
                held: [false; 128],
                out_of_process,
            })
        }
    }

    pub fn shared(&self) -> SharedAudioUnit {
        self.unit.clone()
    }

    fn send(&self, delay: usize, bytes: [u8; 3]) {
        let mut bytes = bytes;
        let time = AUEventSampleTimeImmediate + delay as i64;
        self.midi_block.call((time, 0, 3, NonNull::from(&mut bytes[0])));
    }
}

fn instantiate(desc: AudioComponentDescription, options: AudioComponentInstantiationOptions) -> Result<Retained<AUAudioUnit>, String> {
    let (tx, rx) = mpsc::channel::<Result<SharedAudioUnit, String>>();
    let handler = RcBlock::new(move |unit: *mut AUAudioUnit, error: *mut NSError| {
        let result = match unsafe { Retained::retain(unit) } {
            Some(u) => Ok(SharedAudioUnit(u)),
            None => Err(unsafe { error.as_ref() }
                .map_or("plugin could not be loaded".to_string(), |e| e.localizedDescription().to_string())),
        };
        let _ = tx.send(result);
    });
    unsafe { AUAudioUnit::instantiateWithComponentDescription_options_completionHandler(desc, options, &handler) };
    // Out-of-process launch plus big sample libraries can take a while.
    let unit = rx.recv_timeout(Duration::from_secs(60)).map_err(|_| "plugin took too long to load".to_string())??;
    Ok(unit.0)
}

impl Drop for AudioUnitInstrument {
    fn drop(&mut self) {
        unsafe { self.unit.0.deallocateRenderResources() }
    }
}

impl Instrument for AudioUnitInstrument {
    fn note_on(&mut self, delay: usize, key: u8, velocity: u8) {
        self.held[usize::from(key & 0x7f)] = true;
        self.send(delay, [0x90 | self.channel, key & 0x7f, velocity.clamp(1, 127)]);
    }

    fn note_off(&mut self, delay: usize, key: u8) {
        self.held[usize::from(key & 0x7f)] = false;
        self.send(delay, [0x80 | self.channel, key & 0x7f, 64]);
    }

    fn channel_message(&mut self, delay: usize, status: u8, data1: u8, data2: u8) {
        self.send(delay, [(status & 0xF0) | self.channel, data1 & 0x7f, data2 & 0x7f]);
    }

    fn all_notes_off(&mut self, immediate: bool) {
        for key in 0..128u8 {
            if self.held[usize::from(key)] {
                self.note_off(0, key);
            }
        }
        self.send(0, [0xB0 | self.channel, 64, 0]);
        // All sound off vs. all notes off.
        self.send(0, [0xB0 | self.channel, if immediate { 120 } else { 123 }, 0]);
    }

    fn reset(&mut self) {
        self.held = [false; 128];
        let ch = 0xB0 | self.channel;
        self.send(0, [ch, 120, 0]);
        self.send(0, [ch, 121, 0]);
        self.send(0, [ch, 7, 100]);
        self.send(0, [ch, 11, 127]);
        self.send(0, [0xE0 | self.channel, 0, 64]);
    }

    fn render(&mut self, left: &mut [f32], right: &mut [f32]) -> bool {
        let frames = left.len().min(right.len()).min(MAX_FRAMES as usize);
        let bytes = (frames * 4) as u32;
        let mut list = StereoBufferList {
            number_buffers: 2,
            buffers: [
                AudioBuffer { mNumberChannels: 1, mDataByteSize: bytes, mData: left.as_mut_ptr().cast::<c_void>() },
                AudioBuffer { mNumberChannels: 1, mDataByteSize: bytes, mData: right.as_mut_ptr().cast::<c_void>() },
            ],
        };
        let mut flags = AudioUnitRenderActionFlags(0);
        // SAFETY: plain-old-data struct; only the sample time is meaningful.
        let mut stamp: AudioTimeStamp = unsafe { std::mem::zeroed() };
        stamp.mSampleTime = self.sample_time;
        stamp.mFlags = AudioTimeStampFlags::SampleTimeValid;
        self.sample_time += frames as f64;

        let status = self.render_block.call((
            NonNull::from(&mut flags),
            NonNull::from(&mut stamp),
            frames as u32,
            0,
            NonNull::from(&mut list).cast::<AudioBufferList>(),
            ptr::null_mut(),
        ));
        if status != 0 {
            left[..frames].fill(0.0);
            right[..frames].fill(0.0);
            return false;
        }
        // A unit may point the list at its own buffers instead of ours.
        for (i, dst) in [&mut left[..frames], &mut right[..frames]].into_iter().enumerate() {
            let src = list.buffers[i].mData.cast::<f32>();
            if !src.is_null() && src != dst.as_mut_ptr() {
                // SAFETY: the unit filled `frames` samples at `src`.
                dst.copy_from_slice(unsafe { std::slice::from_raw_parts(src, frames) });
            }
        }
        left[..frames].iter().chain(&right[..frames]).any(|v| v.abs() > 1e-9)
    }
}

/// Open the plugin's editor (or a generic parameter view) in its own window.
/// Must be called on the main thread.
pub fn open_editor(mtm: MainThreadMarker, unit: SharedAudioUnit, title: String) {
    let SharedAudioUnit(au) = unit.clone();
    let handler = RcBlock::new(move |vc: *mut NSViewController| {
        let vc = unsafe { Retained::retain(vc) }.map(SendViewController);
        let unit = unit.clone();
        let title = title.clone();
        // The completion runs on an internal queue; windows belong on main.
        dispatch_main(move || {
            // Move the Send wrappers in whole (not just their fields).
            let (vc, unit) = (vc, unit);
            let mtm = MainThreadMarker::new().expect("main thread");
            let vc = match vc {
                Some(SendViewController(vc)) => vc,
                None => {
                    // No custom UI: Apple's generic parameter editor.
                    let generic = unsafe { AUGenericViewController::new(mtm) };
                    unsafe { generic.setAuAudioUnit(Some(&unit.0)) };
                    Retained::into_super(generic)
                }
            };
            let window = NSWindow::windowWithContentViewController(&vc);
            window.setTitle(&NSString::from_str(&title));
            let mut mask = window.styleMask();
            mask.remove(NSWindowStyleMask::Miniaturizable);
            window.setStyleMask(mask | NSWindowStyleMask::Closable | NSWindowStyleMask::Titled);
            unsafe { window.setReleasedWhenClosed(false) };
            window.center();
            window.makeKeyAndOrderFront(None);
            // Keep the window alive until the process ends or it is reopened.
            EDITORS.with(|e| e.borrow_mut().push(window));
        });
    });
    let _ = mtm;
    unsafe { au.requestViewControllerWithCompletionHandler(&handler) };
}

struct SendViewController(Retained<NSViewController>);
unsafe impl Send for SendViewController {}

thread_local! {
    static EDITORS: std::cell::RefCell<Vec<Retained<NSWindow>>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Run `f` on the main queue (GCD).
fn dispatch_main(f: impl FnOnce() + Send + 'static) {
    extern "C" {
        static _dispatch_main_q: c_void;
        fn dispatch_async_f(queue: *const c_void, context: *mut c_void, work: extern "C" fn(*mut c_void));
    }
    extern "C" fn trampoline(ctx: *mut c_void) {
        let f = unsafe { Box::from_raw(ctx.cast::<Box<dyn FnOnce()>>()) };
        f();
    }
    let boxed: Box<Box<dyn FnOnce()>> = Box::new(Box::new(f));
    unsafe { dispatch_async_f(&_dispatch_main_q, Box::into_raw(boxed).cast(), trampoline) };
}

#[cfg(test)]
mod tests {
    use super::*;

    const DLS: &str = "61756d75-646c7320-6170706c"; // aumu/dls /appl

    #[test]
    fn lists_apple_instruments() {
        let list = list_instruments();
        assert!(list.iter().any(|p| p.id == DLS), "{list:?}");
    }

    #[test]
    fn apple_dls_synth_plays_out_of_process() {
        let mut inst = AudioUnitInstrument::load(DLS, 48_000, 0).expect("load DLS synth");
        let (mut l, mut r) = (vec![0f32; 1024], vec![0f32; 1024]);
        inst.render(&mut l, &mut r);
        inst.note_on(0, 60, 110);
        let mut energy = 0.0;
        for _ in 0..30 {
            inst.render(&mut l, &mut r);
            energy += l.iter().chain(&r).map(|v| v * v).sum::<f32>();
        }
        eprintln!("DLS energy {energy}, out of process: {}", inst.out_of_process);
        assert!(energy > 0.01, "no sound from DLS synth");
    }
}
