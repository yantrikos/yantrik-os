// Yantrik Live: play the relay's HLS while it is really live, and say so plainly when it is not.
// Never a frozen frame: when the picture stops moving, the video is hidden and the card says so.
// It says "since" only for an outage this page saw begin; one already under way when it opened
// has no start time the page could know.
(function () {
  "use strict";
  var SRC = "/live/hls/index.m3u8";
  var RETRY_MS = 15000;   // how often an offline page looks for the stream again
  var STALL_MS = 20000;   // this long without a new frame is offline, not "buffering"

  var video = document.getElementById("video");
  var card = document.getElementById("card");
  var status = document.getElementById("status");
  var title = document.getElementById("card-title");
  var detail = document.getElementById("card-detail");

  var hls = null, retry = null, lastMove = 0, lastTime = -1, offlineSince = null, sawLive = false;

  function clock(d) {
    return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }

  function show(state, heading, text) {
    status.dataset.state = state;
    status.textContent = state === "live" ? "Live" : heading;
    var live = state === "live";
    video.hidden = !live;
    card.hidden = live;
    title.textContent = heading;
    detail.textContent = text;
  }

  function offline() {
    if (sawLive && !offlineSince) offlineSince = new Date();
    show("offline", offlineSince ? "Offline since " + clock(offlineSince) : "Offline",
      "The machine is not streaming right now: an update, maintenance, or a fault. This page looks again every 15 seconds.");
    stop();
    clearTimeout(retry);
    retry = setTimeout(start, RETRY_MS);
  }

  function stop() {
    if (hls) { hls.destroy(); hls = null; }
    video.removeAttribute("src");
    video.load();
  }

  function start() {
    stop();
    lastTime = -1;
    lastMove = Date.now();
    if (window.Hls && window.Hls.isSupported()) {
      hls = new window.Hls({ liveDurationInfinity: true, manifestLoadingMaxRetry: 1, levelLoadingMaxRetry: 2 });
      hls.on(window.Hls.Events.ERROR, function (_e, data) { if (data.fatal) offline(); });
      hls.loadSource(SRC);
      hls.attachMedia(video);
    } else if (video.canPlayType("application/vnd.apple.mpegurl")) {
      video.src = SRC;  // Safari plays HLS itself
    } else {
      show("offline", "Can't play here", "This browser cannot play the stream.");
      return;
    }
    var p = video.play();
    if (p && p.catch) p.catch(function () { /* autoplay waits for the first frame */ });
  }

  video.addEventListener("error", offline);

  // Live means frames are arriving: the playhead moves.
  setInterval(function () {
    if (!hls && !video.src) return;
    if (video.currentTime !== lastTime && !video.paused) {
      lastTime = video.currentTime;
      lastMove = Date.now();
      offlineSince = null;
      sawLive = true;
      if (status.dataset.state !== "live") show("live", "Live", "");
    } else if (Date.now() - lastMove > STALL_MS) {
      offline();
    }
  }, 1000);

  start();
})();
