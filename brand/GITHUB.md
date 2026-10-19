# GitHub — what is still wrong, and the exact commands to fix it

Nothing in this file has been run. GitHub state was deliberately left alone; these are the
calls to make, and the two things that have no API and must be done in a browser.

Today, `github.com/yantrikos` wears YantrikDB's avatar, and `yantrikos/yantrik-os` has
`homepage: null` and no social preview — so every link to the repo unfurls as a grey box
with no picture and no site.

## 1. Repo homepage and description — `gh api`

Requires `gh auth login` as someone with admin on the repo.

```sh
gh api -X PATCH repos/yantrikos/yantrik-os \
  -f homepage='https://www.yantrikos.com' \
  -f description='The AI-native desktop. A Rust shell where every app publishes a graded control surface, and any mind attaches over a socket — the OS never holds its model or its keys.'
```

Topics, while you are there (optional, and it replaces the whole list):

```sh
gh api -X PUT repos/yantrikos/yantrik-os/topics \
  -f names[]='operating-system' \
  -f names[]='desktop-environment' \
  -f names[]='rust' \
  -f names[]='slint' \
  -f names[]='ai-agents' \
  -f names[]='wayland' \
  -f names[]='local-first'
```

Check it took:

```sh
gh api repos/yantrikos/yantrik-os --jq '{homepage, description, topics}'
```

## 2. Repo social preview — web UI only

There is no REST endpoint for the social preview image. Upload it by hand:

> **https://github.com/yantrikos/yantrik-os/settings** → *Social preview* → **Upload an image**

Use **`brand/social-preview-1280x640.png`** (1280×640, under 1 MB — GitHub's stated
requirement is 1280×640 and ≤ 1 MB, and this file is well under that).

This is the picture that appears when the repo is linked on Slack, Discord, X or LinkedIn.

## 3. Org avatar — web UI only

Also no API. The organisation avatar is the one showing YantrikDB's logo:

> **https://github.com/organizations/yantrikos/settings/profile** → *Profile picture* →
> **Upload new picture**

Use **`brand/github-avatar-512.png`** (512×512, the full emblem on `#05070d`). GitHub crops
avatars to a circle; the emblem's gates sit inside it, so nothing of the mark is cut.

While on that page, it is worth setting the org's own homepage to
`https://www.yantrikos.com` and its description to match the repo's.

## 4. The website repo, if it is public

Same two fields, if `yantrikos/yantrik-website` exists on GitHub:

```sh
gh api -X PATCH repos/yantrikos/yantrik-website \
  -f homepage='https://www.yantrikos.com' \
  -f description='The site for Yantrik OS.'
```

## Files to hand to GitHub

| where | file |
|---|---|
| org avatar | `brand/github-avatar-512.png` |
| repo social preview | `brand/social-preview-1280x640.png` |
| anywhere a logo is asked for | `brand/yantrik-emblem.svg`, or `brand/yantrik-icon-512.png` for a square icon |

All of them come from `brand/build.py` via `brand/render.py`. If the mark ever changes, re-run
it and re-upload these two — there is no other copy to chase.
