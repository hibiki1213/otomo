import Player from "@/components/Player";

export default function Home() {
  return (
    <main className="app">
      <Player />
      <p className="footnote">
        音源は読み込み時にオンラインの General MIDI サウンドフォントから取得します。ファイルはブラウザ内で処理され、どこにもアップロードされません。
      </p>
    </main>
  );
}
