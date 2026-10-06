/**
 * Every event in a MIDI file as one readable row: the event list's data. The
 * piano roll draws notes; this is where everything else — bank changes,
 * NRPNs, SysEx, text — can be seen at all.
 */
import type { MidiEvent } from "midi-file";
import { messages } from "./i18n";
import { ccName, drumName, nrpnName, rpnName } from "./midiNames";
import { decodeText } from "./midiText";
import { prettyInstrumentName } from "./gm";
import { noteName } from "./rollSelection";

export type EventCategory = "note" | "control" | "bend" | "program" | "sysex" | "meta";

export interface EventRow {
  /** Seconds from the start. */
  time: number;
  /** Quarter notes from the start. */
  beat: number;
  category: EventCategory;
  /** 0–15, or null for meta events and SysEx. */
  channel: number | null;
  /** The file's track (MTrk) it sits in, 0-based. */
  track: number;
  type: string;
  detail: string;
}

export interface EventListing {
  rows: EventRow[];
  /** Track names as the file gives them, by MTrk index. */
  trackNames: string[];
}

const PERCUSSION_CHANNEL = 9;

interface Timed {
  tick: number;
  track: number;
  seq: number;
  event: MidiEvent;
}

/** Ticks → seconds and quarter notes, following the file's tempo changes. */
function clock(timed: Timed[], ticksPerBeat: number | undefined, ticksPerSecond: number) {
  if (!ticksPerBeat) {
    // SMPTE time: ticks are real time; call a quarter half a second.
    return (tick: number) => ({ time: tick / ticksPerSecond, beat: (tick / ticksPerSecond) * 2 });
  }
  const changes: { tick: number; seconds: number; usPerBeat: number }[] = [
    { tick: 0, seconds: 0, usPerBeat: 500000 },
  ];
  for (const { tick, event } of timed) {
    if (event.type !== "setTempo") continue;
    const last = changes[changes.length - 1];
    const seconds = last.seconds + ((tick - last.tick) / ticksPerBeat) * (last.usPerBeat / 1e6);
    if (tick === last.tick) last.usPerBeat = event.microsecondsPerBeat;
    else changes.push({ tick, seconds, usPerBeat: event.microsecondsPerBeat });
  }
  let i = 0;
  // Rows are asked for in time order, so the segment only moves forward.
  return (tick: number) => {
    while (i + 1 < changes.length && changes[i + 1].tick <= tick) i++;
    const c = changes[i];
    return {
      time: c.seconds + ((tick - c.tick) / ticksPerBeat) * (c.usPerBeat / 1e6),
      beat: tick / ticksPerBeat,
    };
  };
}

const hex = (bytes: ArrayLike<number>, from = 0, to = bytes.length) =>
  Array.from(Array.prototype.slice.call(bytes, from, to) as number[], (b) =>
    b.toString(16).toUpperCase().padStart(2, "0")
  ).join(" ");

const MAKERS: Record<number, string> = {
  0x2f: "Elka / Generalmusic",
  0x40: "Kawai",
  0x41: "Roland",
  0x42: "Korg",
  0x43: "Yamaha",
  0x44: "Casio",
  0x7d: "Non-commercial",
  0x7e: "Universal (non-real-time)",
  0x7f: "Universal (real-time)",
};

/** What a SysEx message is, when it is one of the well-known ones. `data` follows the F0. */
function describeSysex(data: ArrayLike<number>): string {
  const t = messages().events;
  const d = Array.from(data);
  const end = d[d.length - 1] === 0xf7 ? d.length - 1 : d.length;
  if (d[0] === 0x7e && d[2] === 0x09) {
    if (d[3] === 0x01) return t.gmOn;
    if (d[3] === 0x02) return t.gmOff;
    if (d[3] === 0x03) return t.gm2On;
  }
  if (d[0] === 0x7f && d[2] === 0x04 && d[3] === 0x01 && end >= 6) {
    return t.masterVolume(Math.round((((d[5] << 7) | d[4]) / 16383) * 100));
  }
  // Roland GS data set (DT1): address, data, checksum.
  if (d[0] === 0x41 && d[2] === 0x42 && d[3] === 0x12 && end >= 9) {
    const address = hex(d, 4, 7);
    const value = hex(d, 7, end - 1);
    return address === "40 00 7F" && value === "00" ? t.gsReset : t.gsParam(address, value);
  }
  // Yamaha XG parameter change: address, data.
  if (d[0] === 0x43 && (d[1] & 0xf0) === 0x10 && d[2] === 0x4c && end >= 7) {
    const address = hex(d, 3, 6);
    const value = hex(d, 6, end);
    return address === "00 00 7E" && value === "00" ? t.xgOn : t.xgParam(address, value);
  }
  const maker = MAKERS[d[0]];
  return maker ? t.maker(maker) : t.unknownMaker(hex(d, 0, d[0] === 0 ? 3 : 1));
}

function keyLabel(sharps: number, minor: boolean): string {
  const t = messages().events;
  const majors = ["Cb", "Gb", "Db", "Ab", "Eb", "Bb", "F", "C", "G", "D", "A", "E", "B", "F#", "C#"];
  const minors = ["Ab", "Eb", "Bb", "F", "C", "G", "D", "A", "E", "B", "F#", "C#", "G#", "D#", "A#"];
  const i = Math.max(-7, Math.min(7, sharps)) + 7;
  return minor ? `${minors[i]} ${t.minor}` : `${majors[i]} ${t.major}`;
}

const num = (n: number, digits = 2) => String(Math.round(n * 10 ** digits) / 10 ** digits);
const signed = (n: number) => (n > 0 ? `+${num(n)}` : num(n));

/** Read every event of a Standard MIDI File, in time order. */
export async function listEvents(buffer: ArrayBuffer): Promise<EventListing> {
  const { parseMidi } = await import("midi-file");
  const midi = parseMidi(new Uint8Array(buffer));
  const t = messages().events;
  const k = t.kinds;

  const timed: Timed[] = [];
  const trackNames: string[] = midi.tracks.map(() => "");
  midi.tracks.forEach((events, track) => {
    let tick = 0;
    events.forEach((event, seq) => {
      tick += event.deltaTime;
      if (event.type === "trackName" && !trackNames[track]) trackNames[track] = decodeText(event.text).trim();
      if (event.type !== "endOfTrack") timed.push({ tick, track, seq, event });
    });
  });
  // Format 0 keeps everything in one track; across tracks, ties keep file order.
  timed.sort((a, b) => a.tick - b.tick || a.track - b.track || a.seq - b.seq);

  const { ticksPerBeat, framesPerSecond = 30, ticksPerFrame = 80 } = midi.header;
  const at = clock(timed, ticksPerBeat, framesPerSecond * ticksPerFrame);

  // What each channel's data entry sets, its bend range and bank, in time order.
  const state = Array.from({ length: 16 }, () => ({
    rpn: true,
    msb: 127,
    lsb: 127,
    bendRange: 2,
    bank: [0, 0] as [number, number],
  }));
  // Notes still sounding: per track, channel and key, oldest first.
  const open = new Map<number, { note: number; beat: number }[]>();
  const notes: { row: number; key: number; velocity: number; channel: number; beats: number | null }[] = [];

  const rows: EventRow[] = [];
  const push = (row: Omit<EventRow, "time" | "beat">, tick: number) => {
    rows.push({ ...at(tick), ...row });
    return rows.length - 1;
  };

  for (const { tick, track, event: e } of timed) {
    switch (e.type) {
      case "noteOn":
      case "noteOff": {
        const id = (track << 11) | (e.channel << 7) | e.noteNumber;
        if (e.type === "noteOn" && e.velocity > 0) {
          const row = push({ category: "note", channel: e.channel, track, type: k.note, detail: "" }, tick);
          notes.push({ row, key: e.noteNumber, velocity: e.velocity, channel: e.channel, beats: null });
          const queue = open.get(id) ?? [];
          queue.push({ note: notes.length - 1, beat: rows[row].beat });
          open.set(id, queue);
        } else {
          const started = open.get(id)?.shift();
          if (started) notes[started.note].beats = at(tick).beat - started.beat;
        }
        break;
      }
      case "controller": {
        const s = state[e.channel];
        const n = e.controllerType;
        let detail = t.controller(n, ccName(n), e.value);
        if (n === 101 || n === 100) {
          if (!s.rpn) Object.assign(s, { rpn: true, msb: 127, lsb: 127 });
          if (n === 101) s.msb = e.value;
          else s.lsb = e.value;
        } else if (n === 99 || n === 98) {
          if (s.rpn) Object.assign(s, { rpn: false, msb: 127, lsb: 127 });
          if (n === 99) s.msb = e.value;
          else s.lsb = e.value;
        } else if (n === 6 || n === 38 || n === 96 || n === 97) {
          const param = s.rpn
            ? t.rpn(s.msb, s.lsb, rpnName(s.msb, s.lsb))
            : t.nrpn(s.msb, s.lsb, nrpnName(s.msb, s.lsb));
          detail += t.sets(param);
          if (s.rpn && s.msb === 0 && s.lsb === 0) {
            if (n === 6) s.bendRange = e.value;
            else if (n === 38) s.bendRange = Math.floor(s.bendRange) + e.value / 100;
          }
        } else if (n === 0) {
          s.bank[0] = e.value;
        } else if (n === 32) {
          s.bank[1] = e.value;
        }
        push({ category: "control", channel: e.channel, track, type: k.cc, detail }, tick);
        break;
      }
      case "pitchBend": {
        // midi-file gives -8192 … 8191.
        const semitones = (e.value / 8192) * state[e.channel].bendRange;
        push({ category: "bend", channel: e.channel, track, type: k.bend, detail: t.bend(signed(semitones), e.value) }, tick);
        break;
      }
      case "programChange": {
        const [msb, lsb] = state[e.channel].bank;
        const name = e.channel === PERCUSSION_CHANNEL ? messages().midi.drums : prettyInstrumentName(e.programNumber);
        const detail = t.program(name, e.programNumber) + (msb || lsb ? t.bank(msb, lsb) : "");
        push({ category: "program", channel: e.channel, track, type: k.program, detail }, tick);
        break;
      }
      case "channelAftertouch":
        push({ category: "control", channel: e.channel, track, type: k.pressure, detail: t.pressure(e.amount) }, tick);
        break;
      case "noteAftertouch": {
        const name = e.channel === PERCUSSION_CHANNEL ? drumName(e.noteNumber) : noteName(e.noteNumber);
        const detail = t.keyPressure(name, e.noteNumber, e.amount);
        push({ category: "control", channel: e.channel, track, type: k.polyPressure, detail }, tick);
        break;
      }
      case "sysEx":
      case "endSysEx": {
        const bytes = `${e.type === "sysEx" ? "F0 " : ""}${hex(e.data, 0, Math.min(e.data.length, 24))}${e.data.length > 24 ? " …" : ""}`;
        push({ category: "sysex", channel: null, track, type: k.sysex, detail: `${describeSysex(e.data)} · ${bytes}` }, tick);
        break;
      }
      case "setTempo":
        push({ category: "meta", channel: null, track, type: k.tempo, detail: t.tempo(num(60e6 / e.microsecondsPerBeat)) }, tick);
        break;
      case "timeSignature":
        push({ category: "meta", channel: null, track, type: k.timeSignature, detail: `${e.numerator}/${e.denominator}` }, tick);
        break;
      case "keySignature":
        push({ category: "meta", channel: null, track, type: k.keySignature, detail: keyLabel(e.key, e.scale === 1) }, tick);
        break;
      case "text":
      case "lyrics":
      case "marker":
      case "cuePoint":
      case "trackName":
      case "instrumentName":
      case "copyrightNotice": {
        const type = e.type === "copyrightNotice" ? k.copyright : k[e.type];
        const detail = JSON.stringify(decodeText(e.text)).slice(1, -1);
        push({ category: "meta", channel: null, track, type, detail }, tick);
        break;
      }
      case "smpteOffset":
        push(
          {
            category: "meta",
            channel: null,
            track,
            type: k.smpte,
            detail: `${e.hour}:${e.min}:${e.sec}:${e.frame}.${e.subFrame} (${e.frameRate} fps)`,
          },
          tick
        );
        break;
      case "sequencerSpecific":
        push({ category: "meta", channel: null, track, type: k.sequencerSpecific, detail: hex(e.data, 0, Math.min(e.data.length, 24)) }, tick);
        break;
      case "unknownMeta":
        push({ category: "meta", channel: null, track, type: k.other, detail: `FF ${hex([e.metatypeByte])} · ${hex(e.data, 0, Math.min(e.data.length, 24))}` }, tick);
        break;
      default:
        break;
    }
  }

  for (const n of notes) {
    const name = n.channel === PERCUSSION_CHANNEL ? drumName(n.key) : noteName(n.key);
    rows[n.row].detail =
      n.beats == null ? t.noteUnended(name, n.key, n.velocity) : t.note(name, n.key, n.velocity, num(n.beats));
  }
  return { rows, trackNames };
}
