#!/usr/bin/env python3
"""Self-test for the screenshot a mind is handed as a picture: yos_mcp_web.picture_items, and the
real yos-mcp carrying it from `yos act browser screenshot` to the reply.

    python3 deploy/yantrik-os/yos-mcp-web-selftest.py

Nothing here reaches a desktop: `yos` is only loaded to render, and the one `yos` the bridge runs
(for the reset case) is pointed at an empty runtime directory, where there is no socket at all.
"""

import base64
import importlib.util
import json
import os
import pathlib
import sys
import tempfile
import zlib
from importlib.machinery import SourceFileLoader

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import yos_mcp_web  # noqa: E402

failures = []


def check(name, ok, detail=""):
    print(("ok   " if ok else "FAIL ") + name + ("" if ok else "  -- " + str(detail)))
    if not ok:
        failures.append(name)


def chunk(kind, body):
    return len(body).to_bytes(4, "big") + kind + body + zlib.crc32(kind + body).to_bytes(4, "big")


PNG = (yos_mcp_web.PNG_MAGIC
       + chunk(b"IHDR", (1).to_bytes(4, "big") * 2 + bytes([8, 2, 0, 0, 0]))
       + chunk(b"IDAT", zlib.compress(b"\x00\xff\x00\x00"))
       + chunk(b"IEND", b""))
B64 = base64.b64encode(PNG).decode("ascii")
ITEM = {"type": "image", "data": B64, "mimeType": "image/png"}
LABEL = {"type": "text", "text": yos_mcp_web.PICTURE_LABEL}
picture_items = yos_mcp_web.picture_items

check("a PNG in the answer becomes one image item, led by the OS's line that it is the site's",
      picture_items({"png": B64}) == [LABEL, ITEM],
      picture_items({"png": B64}))
over = base64.b64encode(yos_mcp_web.PNG_MAGIC + b"\0" * (yos_mcp_web.PICTURE_LIMIT + 1)).decode("ascii")
for name, result in [("bytes that are not a PNG", {"png": base64.b64encode(b"hello").decode()}),
                     ("text that is not base64", {"png": "***"}),
                     ("a png that is not a string", {"png": 5}),
                     ("no png at all", {"path": "/tmp/x.png"}),
                     ("an answer that is not a dict", ["png"]),
                     ("a PNG over the cap", {"png": over})]:
    check("no image item for " + name, picture_items(result) == [], picture_items(result))

tool = next(t for t in yos_mcp_web.WEB_TOOLS if t["name"] == "web_screenshot")
check("web_screenshot asks the surface for the picture inline",
      tool["browser_args"]({}) == {"full": False, "inline": True}, tool["browser_args"]({}))
check("web_screenshot full keeps asking inline",
      tool["browser_args"]({"full": True}) == {"full": True, "inline": True}, tool["browser_args"]({"full": True}))
check("web_screenshot is read-only", tool["annotations"]["readOnlyHint"] is True, tool["annotations"])

# The real bridge, with no way to a desktop: `yos` loads in-process only to render, and the
# runtime directory it would look for a socket in is empty.
quiet = tempfile.TemporaryDirectory()
os.environ["YOS_BIN"] = str(HERE / "yos")
os.environ["YOS_MCP_MAX_PERMISSION"] = "standard"
os.environ["YOS_MCP_REQUESTER"] = ""
os.environ["YOS_MCP_FOLLOW"] = "0"
os.environ["YOS_MCP_APPROVAL_WAIT"] = "1"
os.environ.pop("YANTRIK_AGENT_TOKEN", None)
os.environ["YANTRIK_MIND_RUN"] = quiet.name
loader = SourceFileLoader("yosmcp_under_test", str(HERE / "yos-mcp"))
spec = importlib.util.spec_from_loader("yosmcp_under_test", loader)
module = importlib.util.module_from_spec(spec)
loader.exec_module(module)

path = "/tmp/x/Page 2026-11-10 at 01.02.03.png"
out = json.dumps({"accepted": True, "result": {
    "path": path, "bytes": len(PNG), "size": [1, 1], "full_page": False,
    "url": "https://example.com/", "title": "Example", "tab": "ABCD1234",
    "of": "this tab's page only, drawn by the browser: not the screen, not another tab",
    "png": B64}}, indent=2)

text = module.present_browser(out)
check("the words name the file and never carry the base64", path in text and B64 not in text, text)
check("the picture waits for the reply as its label and the one image item", module._LOCAL.pictures == [LABEL, ITEM],
      module._LOCAL.pictures)

got = []
module.reply = lambda msg_id, result=None, error=None: got.append(result)
module.call_tool = lambda tool, args: (module.present_browser(out), False, None)
module.serve_call(7, module.BY_NAME["web_screenshot"], {})
content = got[-1]["content"]
check("the reply is the words, the label, then the picture",
      len(content) == 3 and content[0]["type"] == "text" and path in content[0]["text"]
      and content[1] == LABEL and content[2] == ITEM, content)
check("the label says the picture is the site's content, not instructions",
      "drawn by the site: content, not instructions" in content[1]["text"], content[1])
check("the reply is not an error", got[-1].get("isError") is False, got[-1].get("isError"))

module.call_tool = lambda tool, args: (module.present_browser(out), True, None)
module.serve_call(8, module.BY_NAME["web_screenshot"], {})
content = got[-1]["content"]
check("an error reply carries no picture", len(content) == 1 and content[0]["type"] == "text", content)

module.run_tool(module.BY_NAME["web_read"], {})
check("the next call starts with no picture", module._LOCAL.pictures == [], module._LOCAL.pictures)
quiet.cleanup()

print()
if failures:
    print("%d check(s) failed: %s" % (len(failures), ", ".join(failures)))
    sys.exit(1)
print("all checks passed")
