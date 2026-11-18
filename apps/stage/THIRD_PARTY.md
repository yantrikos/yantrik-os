# Stage: third-party licences

Record of what Stage ships or downloads and under what terms. Yantrik OS ships
nothing with a restrictive licence (decision recorded on issue #1139, 8 Nov).

## Voices (Piper models, pinned in voices/voices.json)
Every model is fetched from rhasspy/piper-voices at commit
`e7229f7d33a10e145e109b886d661984201ac3c4` and checked against the sha256 in
voices.json. The licence column quotes the `License:` line of the model card
(`MODEL_CARD` beside the model) as it stands at that commit.

| Stage voice | Model | Model card says | Dataset | Trained | Model card |
|---|---|---|---|---|---|
| claude | en_US-ljspeech-medium (also the OS voice) | `License: public domain` | https://keithito.com/LJ-Speech-Dataset/ | from scratch | [card](https://huggingface.co/rhasspy/piper-voices/blob/e7229f7d33a10e145e109b886d661984201ac3c4/en/en_US/ljspeech/medium/MODEL_CARD) |
| gpt | en_US-john-medium | `License: public domain` | https://librivox.org | finetuned from kristin, which is from scratch | [card](https://huggingface.co/rhasspy/piper-voices/blob/e7229f7d33a10e145e109b886d661984201ac3c4/en/en_US/john/medium/MODEL_CARD) |
| gemini | en_GB-cori-medium | `License: public domain` | https://librivox.org | from scratch | [card](https://huggingface.co/rhasspy/piper-voices/blob/e7229f7d33a10e145e109b886d661984201ac3c4/en/en_GB/cori/medium/MODEL_CARD) |
| grok | en_US-norman-medium | `License: public domain` | https://librivox.org | from scratch | [card](https://huggingface.co/rhasspy/piper-voices/blob/e7229f7d33a10e145e109b886d661984201ac3c4/en/en_US/norman/medium/MODEL_CARD) |

Why these four: they are the single-speaker voices whose cards state a
permissive licence and whose training starts from scratch or from another
public-domain voice. Most CC0 / CC BY / Apache-2.0 voices in the repository
(joe, mike, sam, alba, aru, vctk, libritts_r and others) say "Finetuned from
U.S. English lessac voice", so they inherit the Blizzard 2013 Lessac lineage
and were left out. Left out for their own terms: ryan (CC BY-NC-SA 4.0),
hfc_* (CC BY-NC-SA), l2arctic (CC BY-NC), semaine (CC BY-NC-SA), and amy, alan,
danny, kusal and jenny_dioco, whose cards only say "See URL".

Replaced in this change: `en_US-lessac-medium` (Blizzard 2013 licence) and
`en_US-ryan-medium` (CC BY-NC-SA 4.0), `en_GB-alan-medium` and `en_US-amy-medium`
(card: "See URL").

Credit line for videos: "Voices: Piper TTS (rhasspy/piper-voices); public-domain
LibriVox and LJ Speech recordings".

## Engines and tools
- Piper and espeak-ng data (`/opt/yantrik/models/tts`): run as separate programs
  and not linked into Stage; they travel in the ISO as their own packages with
  their own licence files.
- Chrome for Testing headless shell 158.0.8127.2 (linux64), fetched by install.sh
  from storage.googleapis.com/chrome-for-testing-public, zip sha256 in
  `browser.zip.sha256`, binary sha256 in `browser.sha256`. Chromium BSD-style
  licence plus the third-party licences shipped inside the zip.
- puppeteer-core 25.14.0: Apache-2.0. Drives the browser; npm lockfile pins every
  package by sha512.
- esbuild 0.28.1: MIT. Bundles the template. react / react-dom 18.3.1: MIT.
- ffmpeg: from the OS; its own licence applies.
- Remotion is no longer used (removed in this change: its licence is
  source-available, not open).

## Font
- Fredoka: SIL Open Font License 1.1, text in fonts/OFL-Fredoka.txt.
