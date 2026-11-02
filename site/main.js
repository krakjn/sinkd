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

const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
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
