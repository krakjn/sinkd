// Marine snow carried on a current, plus a few rising bubbles, behind the hero.
(() => {
  const canvas = document.getElementById("current");
  if (!canvas || matchMedia("(prefers-reduced-motion: reduce)").matches) return;
  const ctx = canvas.getContext("2d");
  let w = 0;
  let h = 0;
  let snow = [];
  let bubbles = [];
  let running = true;

  const rand = (a, b) => a + Math.random() * (b - a);

  function resize() {
    const dpr = Math.min(devicePixelRatio || 1, 2);
    w = canvas.clientWidth;
    h = canvas.clientHeight;
    canvas.width = Math.round(w * dpr);
    canvas.height = Math.round(h * dpr);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    snow = Array.from({ length: Math.min(170, Math.round((w * h) / 8000)) }, () => ({
      x: rand(0, w),
      y: rand(0, h),
      r: rand(0.5, 2),
      v: rand(0.15, 0.7),
      p: rand(0, Math.PI * 2),
      a: rand(0.12, 0.5),
    }));
    bubbles = Array.from({ length: 7 }, () => newBubble(rand(0, h)));
  }

  function newBubble(y) {
    return { x: rand(0.1, 0.9) * w, y, r: rand(1.5, 4.5), v: rand(0.35, 0.9), p: rand(0, 6) };
  }

  function frame(t) {
    if (!running) return;
    ctx.clearRect(0, 0, w, h);
    const gust = 1 + 0.45 * Math.sin(t / 2600);
    for (const s of snow) {
      s.x += s.v * gust * (1 + s.r * 0.25);
      s.y += Math.sin(t / 1100 + s.p + s.x / 140) * 0.18;
      if (s.x > w + 4) { s.x = -4; s.y = rand(0, h); }
      ctx.globalAlpha = s.a;
      ctx.fillStyle = "#e6fbf6";
      ctx.beginPath();
      ctx.arc(s.x, s.y, s.r, 0, Math.PI * 2);
      ctx.fill();
    }
    ctx.lineWidth = 1;
    ctx.strokeStyle = "#dffaf4";
    for (let i = 0; i < bubbles.length; i++) {
      const b = bubbles[i];
      b.y -= b.v;
      b.x += Math.sin(t / 500 + b.p) * 0.35 + 0.25 * gust;
      if (b.y < -10) bubbles[i] = newBubble(h + 10);
      ctx.globalAlpha = 0.45;
      ctx.beginPath();
      ctx.arc(b.x, b.y, b.r, 0, Math.PI * 2);
      ctx.stroke();
    }
    ctx.globalAlpha = 1;
    requestAnimationFrame(frame);
  }

  resize();
  addEventListener("resize", resize);
  new IntersectionObserver(([entry]) => {
    const was = running;
    running = entry.isIntersecting;
    if (running && !was) requestAnimationFrame(frame);
  }).observe(canvas);
  requestAnimationFrame(frame);
})();
