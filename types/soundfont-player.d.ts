declare module "soundfont-player" {
  export interface PlayOptions {
    gain?: number;
    duration?: number;
    attack?: number;
    decay?: number;
    sustain?: number;
    release?: number;
    loop?: boolean;
  }

  export interface SoundfontPlayer {
    play(
      note: string | number,
      when?: number,
      options?: PlayOptions
    ): AudioBufferSourceNode;
    stop(when?: number): void;
    connect(destination: AudioNode): void;
    on(event: string, callback: (...args: unknown[]) => void): void;
  }

  export interface InstrumentOptions {
    soundfont?: "MusyngKite" | "FluidR3_GM" | string;
    format?: "mp3" | "ogg";
    destination?: AudioNode;
    gain?: number;
    notes?: Array<number | string>;
    nameToUrl?: (name: string, soundfont: string, format: string) => string;
  }

  export function instrument(
    ac: AudioContext,
    name: string,
    options?: InstrumentOptions
  ): Promise<SoundfontPlayer>;

  const Soundfont: {
    instrument: typeof instrument;
  };

  export default Soundfont;
}
