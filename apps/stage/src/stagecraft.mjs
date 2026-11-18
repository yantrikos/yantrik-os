// What each set offers an episode: props that can change state, and named marks to walk to.
// Read by the lint and timeline (node) and by the template (browser), so there is one list.
export default {
  "sfx": {
    "record-beep": {"at": "start", "volume": 0.55},
    "rimshot": {"at": "end", "volume": 0.6},
    "power-down": {"at": "start", "volume": 0.6},
    "alarm": {"at": "start", "volume": 0.4},
    "door": {"at": "start", "volume": 0.6},
    "typing": {"at": "start", "volume": 0.5},
    "coffee-pour": {"at": "start", "volume": 0.55},
    "crowd-gasp": {"at": "end", "volume": 0.55},
    "laugh": {"at": "end", "volume": 0.4}
  },
  "everyMarks": {"left": 240, "center": 960, "right": 1680},
  "sets": {
    "server-room": {
      "marks": {"dashboard": 640, "cables": 900, "rack": 1460, "door": 1800},
      "props": {
        "alert-lamp": {
          "about": "the red lamp on the rack; its label plate can say anything",
          "states": ["blink", "steady", "off"], "default": "blink", "text": true, "defaultText": "ALERT", "maxText": 12,
          "insert": {"cx": 1432, "cy": 196, "z": 3.7}
        },
        "dashboard": {
          "about": "the wall screen; text is its headline, state sets the colour",
          "states": ["ok", "warn", "alarm"], "default": "ok", "text": true, "defaultText": "ALL SYSTEMS OPERATIONAL", "maxText": 28,
          "insert": {"cx": 370, "cy": 330, "z": 2.8}
        },
        "rack-doors": {
          "about": "the glass door of the rack under the lamp",
          "states": ["closed", "open"], "default": "closed", "text": false,
          "insert": {"cx": 1432, "cy": 400, "z": 2.1}
        },
        "cable-mess": {
          "about": "the pile of cables at the foot of the racks",
          "states": ["tangled", "pulled"], "default": "tangled", "text": false,
          "insert": {"cx": 900, "cy": 690, "z": 3.0}
        }
      }
    },
    "break-room": {
      "marks": {"window": 840, "coffee": 1290, "vending": 1700},
      "props": {
        "coffee-machine": {
          "about": "the coffee machine; its little screen can say anything",
          "states": ["idle", "brewing", "empty"], "default": "idle", "text": true, "defaultText": "", "maxText": 10,
          "insert": {"cx": 1470, "cy": 392, "z": 3.0}
        },
        "vending-machine": {
          "about": "the vending machine; text is what its keypad shows",
          "states": ["idle", "stuck", "off"], "default": "idle", "text": true, "defaultText": "A4", "maxText": 6,
          "insert": {"cx": 1725, "cy": 430, "z": 2.0}
        },
        "wall-clock": {
          "about": "the wall clock; text is the time, like 4:59",
          "states": ["stopped", "ticking", "racing"], "default": "stopped", "text": true, "defaultText": "4:59", "maxText": 5,
          "insert": {"cx": 1330, "cy": 230, "z": 3.6}
        }
      }
    },
    "rooftop": {
      "marks": {"sign": 1100, "antenna": 1560, "edge": 1350},
      "props": {
        "antenna": {
          "about": "the radio mast with a beacon on top",
          "states": ["blink", "steady", "off"], "default": "blink", "text": false,
          "insert": {"cx": 1700, "cy": 310, "z": 3.0}
        },
        "neon-sign": {
          "about": "the neon sign on the far building; text is the words on it",
          "states": ["on", "flicker", "off"], "default": "on", "text": true, "defaultText": "OPEN LATE", "maxText": 12,
          "insert": {"cx": 1080, "cy": 340, "z": 2.6}
        },
        "string-lights": {
          "about": "the string of party lights overhead",
          "states": ["on", "flicker", "off"], "default": "on", "text": false,
          "insert": {"cx": 960, "cy": 200, "z": 2.0}
        }
      }
    }
  }
}
;
