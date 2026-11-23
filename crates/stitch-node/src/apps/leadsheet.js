// The leadsheet music stack for apps (`/_lib/leadsheet.js`), as edge talk
// loads it: a memoized `window.initLeadsheet()` resolving to the wasm
// compiler (check, render, fmt, compress_midi, compress_jsonl), SpessaSynth's
// classes, and `createSynth(audioCtx)`, which wires the AudioWorklet
// processor and the bundled GM soundfont (GeneralUser GS). The `leadsheet`
// and `spessasynth` modules come through the page's import map (Stitch adds
// it); the wasm, the processor (through a blob URL) and the soundfont are
// fetched from /_lib/.
window.initLeadsheet = (function () {
  var promise = null;
  return function () {
    if (!promise) {
      promise = (async function () {
        var ls = await import('leadsheet');
        await ls.default({ module_or_path: '/_lib/leadsheet_wasm_bg.wasm' });
        var ss = await import('spessasynth');
        return {
          check: ls.check, render: ls.render, fmt: ls.fmt,
          compress_midi: ls.compress_midi, compress_jsonl: ls.compress_jsonl,
          WorkletSynthesizer: ss.WorkletSynthesizer, Sequencer: ss.Sequencer,
          createSynth: async function (audioCtx) {
            var code = await (await fetch('/_lib/spessasynth_processor.min.js')).text();
            var blobURL = URL.createObjectURL(new Blob([code], { type: 'application/javascript' }));
            try {
              await audioCtx.audioWorklet.addModule(blobURL);
            } finally {
              URL.revokeObjectURL(blobURL);
            }
            var synth = new ss.WorkletSynthesizer(audioCtx);
            synth.connect(audioCtx.destination);
            var sf = await (await fetch('/_lib/GeneralUserGS.sf3')).arrayBuffer();
            await synth.soundBankManager.addSoundBank(sf, 'main');
            await synth.isReady;
            return synth;
          }
        };
      })();
    }
    return promise;
  };
})();
