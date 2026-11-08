# Stage: third-party licences

Record of what Stage ships or downloads and under what terms. Review before
any commercial or public distribution of the OS image or of rendered video.

## Voices (Piper models, pinned in voices/voices.json)
| Voice | Model | Licence | Source |
|---|---|---|---|
| claude | en_US-lessac-medium | Blizzard 2013 Lessac licence | https://www.cstr.ed.ac.uk/projects/blizzard/2013/lessac_blizzard2013/license.html |
| gpt | en_US-ryan-medium | **CC BY-NC-SA 4.0, non-commercial** | https://www.kaggle.com/datasets/roholazandie/ryanspeech |
| grok | en_GB-alan-medium | per dataset page (MycroftAI mimic3-voices) | https://github.com/MycroftAI/mimic3-voices/blob/master/voices/en_UK/apope_low |
| gemini | en_US-amy-medium | per dataset page (MycroftAI mimic3-voices) | https://github.com/MycroftAI/mimic3-voices |

Models come from rhasspy/piper-voices at the commit pinned in voices.json and
are verified by sha256 at install. Open decision for the owner: the `ryan` voice
is share-alike and non-commercial; replace it before Stage output is sold or
ships in a commercial image. Credit line for videos that use these voices:
"Voices: Piper TTS (rhasspy/piper-voices); datasets as listed in THIRD_PARTY.md".

## Engines
- Piper and espeak-ng data (`/opt/yantrik/models/tts`): GPL. They run as separate
  programs and are not linked into Stage; they travel in the ISO as their own
  packages with their own licence files.
- Remotion 4.0.541: source-available under the Remotion License, not OSI-open.
  Companies above the free-tier threshold need a paid licence. The licence-key
  path is unused; this needs a decision before commercial use of Stage.
- Headless Chrome (chrome-headless-shell, fetched by `remotion browser ensure`,
  checksum pinned in `browser.sha256`): BSD-style Chromium licence.
- ffmpeg: from the OS; its own licence applies.

## Font
- Fredoka: SIL Open Font License 1.1, text in fonts/OFL-Fredoka.txt.
