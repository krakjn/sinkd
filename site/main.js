const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
const root = document.documentElement;

document.querySelectorAll("[data-copy]").forEach((btn) => {
  btn.addEventListener("click", async () => {
    const node = document.querySelector(btn.getAttribute("data-copy"));
    try {
      await navigator.clipboard.writeText(node.textContent);
    } catch {
      return;
    }
    const label = btn.querySelector(".copy-label") ?? btn;
    const prev = label.textContent;
    label.textContent = "Copied";
    btn.classList.add("done");
    setTimeout(() => {
      label.textContent = prev;
      btn.classList.remove("done");
    }, 1400);
  });
});

const reveal = document.querySelectorAll("[data-reveal]");
if (reduce || !("IntersectionObserver" in window)) {
  reveal.forEach((el) => el.classList.add("in"));
} else {
  const io = new IntersectionObserver(
    (entries) => {
      entries.forEach((entry) => {
        if (!entry.isIntersecting) return;
        entry.target.classList.add("in");
        io.unobserve(entry.target);
      });
    },
    { threshold: 0.2 },
  );
  reveal.forEach((el) => io.observe(el));
}

// Coral in a current: inline the logo so the branches can move apart from the anchor.
const CORAL = new Set([4, 5, 10, 11, 20, 21, 22, 23, 24, 26, 34, 35, 36, 37, 42, 43].map((n) => `path${n}`));
const mark = document.querySelector("#mark");
fetch("assets/sinkd-slim.svg")
  .then((res) => (res.ok ? res.text() : Promise.reject(res.status)))
  .then((text) => {
    const svg = new DOMParser().parseFromString(text, "image/svg+xml").documentElement;
    if (svg.nodeName !== "svg") return;
    svg.removeAttribute("width");
    svg.removeAttribute("height");
    svg.querySelectorAll("g[fill]").forEach((g) => g.setAttribute("fill", "currentColor"));
    mark.replaceChildren(svg);
    mark.classList.add("live");
    svg.querySelectorAll("path").forEach((p) => {
      if (!CORAL.has(p.id)) return;
      const b = p.getBBox();
      // Path space is 10x the viewBox with y flipped: bigger y is higher on screen.
      const height = (b.y + b.height / 2) / 10240;
      p.classList.add("coral", b.x + b.width / 2 < 5120 ? "l" : "r");
      p.style.setProperty("--d", `${(-(1 - height) * 2.4).toFixed(2)}s`);
    });
  })
  .catch(() => {});

// Light rays: uneven spacing and clocks, and a slight lean toward the pointer.
const rays = document.querySelector(".rays");
if (rays) {
  const rand = (a, b) => a + Math.random() * (b - a);
  [...rays.children].forEach((ray, i, all) => {
    const slot = 100 / (all.length + 1);
    ray.style.setProperty("--x", `${(slot * (i + 1) + rand(-slot / 3, slot / 3) - 4).toFixed(1)}%`);
    ray.style.setProperty("--w", `${rand(4, 13).toFixed(1)}vw`);
    const dur = rand(28, 46);
    ray.style.setProperty("--dur", `${dur.toFixed(1)}s`);
    ray.style.setProperty("--del", `${(-rand(0, dur)).toFixed(1)}s`);
    ray.style.setProperty("--tilt", `${rand(-2.5, 2.5).toFixed(2)}deg`);
  });

  if (!reduce) {
    let target = -11;
    let lean = -11;
    let lastMove = -1e9;
    let visible = false;
    let raf = 0;
    addEventListener(
      "pointermove",
      (e) => {
        target = -14 + (e.clientX / innerWidth) * 8;
        lastMove = performance.now();
      },
      { passive: true },
    );
    const tick = (t) => {
      if (t - lastMove > 2500) target = -10.5 + Math.sin(t / 7000) * 3;
      lean += (target - lean) * 0.025;
      rays.style.setProperty("--lean", `${lean.toFixed(2)}deg`);
      raf = visible ? requestAnimationFrame(tick) : 0;
    };
    new IntersectionObserver(([entry]) => {
      visible = entry.isIntersecting;
      if (visible && !raf) raf = requestAnimationFrame(tick);
    }).observe(rays.parentElement);
  }
}

// The dive: scroll depth drives the water color, the depth chip, and the section depth labels.
const MAX_M = 200;
const STOPS = [
  [28, 90, 104],
  [24, 44, 56],
  [4, 8, 11],
];
const read = document.querySelector("#depth-read");
const labels = document.querySelectorAll("[data-depth]");
const digits = (m) => String(Math.round(m)).padStart(3, "0");
const pad = (m) => `${digits(m)} m`;
const mix = (a, b, t) => a.map((v, i) => Math.round(v + (b[i] - v) * t));
const maxScroll = () => Math.max(1, root.scrollHeight - innerHeight);

function dive() {
  const d = Math.min(1, Math.max(0, scrollY / maxScroll()));
  const c = d < 0.5 ? mix(STOPS[0], STOPS[1], d * 2) : mix(STOPS[1], STOPS[2], (d - 0.5) * 2);
  root.style.setProperty("--depth", d.toFixed(4));
  root.style.setProperty("--water", `rgb(${c.join(",")})`);
  read.textContent = digits(d * MAX_M);
}

function labelDepths() {
  labels.forEach((el) => {
    const top = el.closest("section").offsetTop;
    el.textContent = pad(Math.min(1, Math.max(0, top / maxScroll())) * MAX_M);
  });
}

let ticking = false;
addEventListener(
  "scroll",
  () => {
    if (ticking) return;
    ticking = true;
    requestAnimationFrame(() => {
      dive();
      ticking = false;
    });
  },
  { passive: true },
);
// Sonar: a dot under the nav link for the section in the middle of the screen.
const navLinks = [...document.querySelectorAll(".nav a")];
const sonar = document.querySelector(".sonar");
let sonarAt = null;

function placeSonar(link, ping) {
  sonarAt = link;
  navLinks.forEach((a) => a.classList.toggle("on", a === link));
  if (!link) {
    sonar.style.opacity = "0";
    return;
  }
  sonar.style.transform = `translateX(${link.offsetLeft + link.offsetWidth / 2 - 2.5}px)`;
  sonar.style.opacity = "1";
  if (ping) {
    sonar.classList.remove("ping");
    void sonar.offsetWidth;
    sonar.classList.add("ping");
  }
}

if (sonar && "IntersectionObserver" in window) {
  const byId = new Map(navLinks.map((a) => [a.hash.slice(1), a]));
  const watch = new IntersectionObserver(
    (entries) => {
      entries.forEach((entry) => {
        if (!entry.isIntersecting) return;
        const link = byId.get(entry.target.id) ?? null;
        if (link !== sonarAt) placeSonar(link, true);
      });
    },
    { rootMargin: "-45% 0px -50% 0px" },
  );
  document.querySelectorAll(".hero, main > section").forEach((s) => watch.observe(s));
}

addEventListener("resize", () => {
  labelDepths();
  dive();
  if (sonar) placeSonar(sonarAt, false);
});
addEventListener("load", labelDepths);
document.fonts?.ready.then(labelDepths);
labelDepths();
dive();

document.querySelectorAll(".card").forEach((card) => {
  card.addEventListener("pointermove", (e) => {
    const r = card.getBoundingClientRect();
    card.style.setProperty("--mx", `${e.clientX - r.left}px`);
    card.style.setProperty("--my", `${e.clientY - r.top}px`);
  });
});

if (reduce) document.querySelector("#diagram")?.pauseAnimations();

// Sync demo. Mirrors real behavior: the hub wins, the other edit goes to conflicts/.
const demo = document.querySelector(".demo");
if (demo) {
  const lane = demo.querySelector(".lane");
  const hub = demo.querySelector(".hub-node");
  const laptop = demo.querySelector("#laptop [data-file]");
  const desktop = demo.querySelector("#desktop [data-file]");
  const tray = demo.querySelector("#tray");
  const status = document.querySelector("#demo-status");
  const buttons = document.querySelectorAll("[data-act]");
  const START = "09:14";
  const now = () => new Date().toTimeString().slice(0, 5);
  let busy = false;

  const flash = (el, cls) => {
    el.classList.remove("edited", "synced", "ping");
    void el.offsetWidth;
    el.classList.add(cls);
  };
  const setFile = (li, time, cls) => {
    li.querySelector("time").textContent = time;
    flash(li, cls);
  };
  const ping = () => flash(hub, "ping");
  const travel = (from, to, warm) => {
    const p = document.createElement("i");
    p.className = warm ? "packet warm" : "packet";
    lane.append(p);
    const anim = p.animate([{ left: `${from}%` }, { left: `${to}%` }], {
      duration: reduce ? 0 : 750,
      easing: "ease-in-out",
    });
    return anim.finished.then(() => p.remove());
  };

  const acts = {
    async edit() {
      const t = now();
      setFile(laptop, t, "edited");
      status.textContent = "laptop saved notes.md and pushes it to the hub.";
      await travel(0, 50);
      ping();
      status.textContent = "The hub has it. desktop pulls.";
      await travel(50, 100);
      setFile(desktop, t, "synced");
      status.textContent = `Both machines have notes.md from ${t}.`;
    },
    async conflict() {
      const t = now();
      setFile(laptop, t, "edited");
      setFile(desktop, t, "edited");
      status.textContent = "Both machines edited notes.md before syncing.";
      await Promise.all([travel(0, 50), travel(100, 50, true)]);
      ping();
      tray.querySelector(".empty")?.remove();
      const li = document.createElement("li");
      li.className = "aside";
      li.innerHTML = `<span></span><time>${t}</time>`;
      li.firstChild.textContent = `notes.md.sinkd-conflict-desktop-${t.replace(":", "")}`;
      tray.append(li);
      status.textContent = "The laptop's push reached the hub first. desktop's edit is set aside, not lost.";
      await travel(50, 100);
      setFile(desktop, t, "synced");
      status.textContent = "desktop now matches the hub. Its own edit is in conflicts/.";
    },
    async reset() {
      [laptop, desktop].forEach((li) => (li.querySelector("time").textContent = START));
      tray.innerHTML = '<li class="empty">nothing set aside</li>';
      status.textContent = "Both machines have the same notes.md.";
    },
  };

  buttons.forEach((btn) =>
    btn.addEventListener("click", async () => {
      if (busy) return;
      busy = true;
      buttons.forEach((b) => (b.disabled = true));
      try {
        await acts[btn.dataset.act]();
      } finally {
        busy = false;
        buttons.forEach((b) => (b.disabled = false));
      }
    }),
  );
}
