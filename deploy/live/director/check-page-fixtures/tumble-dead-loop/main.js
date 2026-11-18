const canvas = document.getElementById("world");
const ctx = canvas.getContext("2d");
const shapes = [];
const demo = new URLSearchParams(location.search).get("demo") === "1";

function resize() {
  canvas.width = canvas.clientWidth;
  canvas.height = canvas.clientHeight;
}

function spawn() {
  shapes.push({ x: Math.random() * canvas.width, y: 0, vx: 0, vy: 0, size: 10 + Math.random() * 30 });
  document.getElementById("count").textContent = shapes.length + " shapes";
}

function frame() {
  ctx.fillStyle = "#10141c";
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  for (const s of shapes) {
    s.vy += 0.5;
    s.y += s.vy;
    ctx.fillStyle = "#f5a623";
    ctx.fillRect(s.x, s.y, s.size, s.size);
  }
  // The bug: no requestAnimationFrame(frame) here, so this runs once, before any shape exists.
}

document.getElementById("clear").onclick = () => { shapes.length = 0; };
resize();
if (demo) setInterval(spawn, 1000);
requestAnimationFrame(frame);
