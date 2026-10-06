/** What MIDI numbers are called, in the current language. */
import { messages } from "./i18n";

const FIRST_DRUM = 27;

/** The percussion sound on `key` (General MIDI, GM2 at the edges). */
export function drumName(key: number): string {
  const { drums, drumKey } = messages().names;
  return drums[key - FIRST_DRUM] ?? drumKey(key);
}

/** A controller's name, or "" for one the standard leaves open. */
export function ccName(n: number): string {
  return messages().names.cc[n] ?? "";
}

/** A registered parameter's name, or "". */
export function rpnName(msb: number, lsb: number): string {
  return messages().names.rpn[`${msb}/${lsb}`] ?? "";
}

/** A GS/XG non-registered parameter's name, or "". */
export function nrpnName(msb: number, lsb: number): string {
  const { nrpn, drumNrpn, drumParam } = messages().names;
  const named = nrpn[`${msb}/${lsb}`];
  if (named) return named;
  const what = drumNrpn[msb];
  return what ? drumParam(drumName(lsb), what) : "";
}
