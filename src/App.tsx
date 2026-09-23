import Player from "@/components/Player";

export default function App() {
  return (
    <main className="app">
      <Player />
      <p className="footnote">
        音源は初回のみオンラインの General MIDI サウンドフォントから取得し、以降はローカルにキャッシュします。MIDI ファイルはこの端末内で処理されます。
      </p>
    </main>
  );
}
