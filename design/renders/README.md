# Reference renders

Target pictures for the UI overhaul, rendered with GPT image models in the direction Pranab chose (2 Oct 2026). They are pictures of the goal, not specs: when a render and `design/minds-surfaces-spec-2026-10-02.md` disagree, the spec wins. Sizes in a render are approximate; image models draw bars and docks too tall.

| File | Shows | Read it for |
|---|---|---|
| `01-desktop-colour-system-grounded-dock.jpg` | The desktop with the chosen dock and colour system | The **authoritative look**: the grounded, centred dock of full-colour app tiles; the soft-blue accent on the primary button, active tile and slider; lighter tiles; the 36px top bar with the Minds chip, the centred clock and the right-side indicators |
| `02-today-and-wifi-laptop.jpg` | Today (clock popover) and the Wi‑Fi list on a laptop | The Today panel's layout and the network popover. Its bottom bar is an older taskbar; use the dock from 01 |
| `03-lock-screen.jpg` | The lock screen | Lock-screen layout: clock, date, name, password field, status line |
| `04-chat.jpg` | The chat panel | Chat v2 (built). Buttons follow 01's colour system |
| `05-agents-workroom.jpg` | The Agents workroom | The workroom (built) |
| `06-alt-tab.jpg` | The Super+Tab overview card | The shell's switcher card (built); Alt+Tab itself is labwc's native switcher, themed |
| `07-dock-overloaded.jpg` | The dock with many apps and a window list | Dock paging and the window list. Its dock is full width; use 01's centred dock |

The default wallpaper is `crates/yantrik-ui-slint/ui/wallpapers/lake.jpg`. It is an original image generated for Yantrik OS (gpt-image-2, 2 Oct 2026), not a stock photo, so it can ship without third-party licence terms. Its source render (3840x2160) had model grain in the sky and water, so the shipped file is made from it with `scripts/wallpaper-clean.py <source.png> lake`, which smooths flat areas with an edge-preserving filter, dithers, and writes the 2560x1600 wallpaper and its 320x200 preview together. Make any new wallpaper the same way, from a source at least 3840x2160, and don't ask the image model for "film grain".
