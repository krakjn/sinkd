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
    const prev = btn.textContent;
    btn.textContent = "Copied";
    setTimeout(() => {
      btn.textContent = prev;
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

const ver = document.querySelector("#version");
fetch("https://api.github.com/repos/octo-works/sinkd/releases/latest")
  .then((res) => (res.ok ? res.json() : Promise.reject(res.status)))
  .then((rel) => {
    ver.textContent = rel.tag_name;
    ver.hidden = false;
    ver.href = rel.html_url;
  })
  .catch(() => {});

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

// The dive: scroll depth drives the water color, the gauge, and the section depth labels.
const MAX_M = 200;
const STOPS = [
  [28, 90, 104],
  [24, 44, 56],
  [4, 8, 11],
];
const read = document.querySelector("#depth-read");
const labels = document.querySelectorAll("[data-depth]");
const pad = (m) => `${String(Math.round(m)).padStart(3, "0")} m`;
const mix = (a, b, t) => a.map((v, i) => Math.round(v + (b[i] - v) * t));
const maxScroll = () => Math.max(1, root.scrollHeight - innerHeight);

function dive() {
  const d = Math.min(1, Math.max(0, scrollY / maxScroll()));
  const c = d < 0.5 ? mix(STOPS[0], STOPS[1], d * 2) : mix(STOPS[1], STOPS[2], (d - 0.5) * 2);
  root.style.setProperty("--depth", d.toFixed(4));
  root.style.setProperty("--water", `rgb(${c.join(",")})`);
  read.textContent = pad(d * MAX_M);
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
addEventListener("resize", () => {
  labelDepths();
  dive();
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
