/* ── 槽 01 的三个候选，全部可交互；内容是同一个真实场景 ── */
(function () {
  var EASE = "cubic-bezier(0.23, 1, 0.32, 1)";

  /* —— 候选 A：推荐卡（Beautiful UI RecommendationCard 的结构） —— */
  (function recCard() {
    var root = document.querySelector('[data-demo="rec"]');
    if (!root) return;
    var OPTIONS = [
      { key: "card",
        body: '按 <span class="ent"><span class="mg">规</span><span class="nm">项目视觉规范</span></span> 的写法，价格与标题左端对齐，影响 <span class="vp">3 个文件</span>',
        short: "卡片左边缘 · 规范里写过", signal: 3, tone: "var(--status-success)", label: "有明确依据", cta: "就按这个", variant: "acc" },
      { key: "img",
        body: '价格跟随主图起点，与卡片留白错开。规范里没写过这一种，<span class="vp neu">需要你定</span>',
        short: "图片左边缘", signal: 2, tone: "var(--status-warning)", label: "需要你定", cta: "改用这个", variant: "ink" },
      { key: "keep",
        body: '这一轮先不动对齐，只改行距，价格位置留到下一轮再说。',
        short: "先不动对齐", signal: 0, tone: "var(--text-dim)", label: "没有依据", cta: "先不动", variant: "ink" }
    ];
    var sel = 0, open = false, done = false;
    var body = root.querySelector(".rc-body"), drawer = root.querySelector(".rc-drawer");
    var list = root.querySelector(".rc-altlist"), lvl = root.querySelector(".lvl");
    var acts = root.querySelector(".acts");

    function meter(signal, tone) {
      var out = '<span class="meter">';
      for (var i = 0; i < 3; i++) out += '<i style="background:' + (i < signal ? tone : "var(--border-strong)") + '"></i>';
      return out + "</span>";
    }
    function draw() {
      var a = OPTIONS[sel];
      body.innerHTML = a.body;
      body.style.animation = "none"; void body.offsetWidth; body.style.animation = "rc-fade 180ms ease-out both";
      list.innerHTML = OPTIONS.map(function (o, i) { return { o: o, i: i }; })
        .filter(function (x) { return x.i !== sel; })
        .map(function (x) {
          return '<button type="button" class="rc-alt" data-i="' + x.i + '">' + meter(x.o.signal, x.o.tone) +
            '<span class="t">' + x.o.short + '</span><span class="n">' + x.o.label + "</span></button>";
        }).join("");
      list.querySelectorAll(".rc-alt").forEach(function (b) {
        b.addEventListener("click", function () { sel = Number(b.getAttribute("data-i")); done = false; draw(); });
      });
      drawer.classList.toggle("on", open && !done);
      lvl.innerHTML = meter(a.signal, a.tone) + a.label;
      acts.innerHTML = done
        ? '<button type="button" class="pbtn ok" disabled>已采纳 · ' + a.short.split(" · ")[0] + "</button>"
        : '<button type="button" class="pbtn sec" data-act="alts" aria-expanded="' + open + '">其他做法</button>' +
          '<button type="button" class="pbtn ' + a.variant + '" data-act="take">' + a.cta + "</button>";
      var alts = acts.querySelector('[data-act="alts"]');
      if (alts) alts.addEventListener("click", function () { open = !open; draw(); });
      var take = acts.querySelector('[data-act="take"]');
      if (take) take.addEventListener("click", function () { done = true; open = false; draw(); });
    }
    draw();
  })();

  /* —— 候选 B：gogoke 原生问题卡（多题排队 + 备注 + 统一提交） —— */
  (function ruiCard() {
    var root = document.querySelector('[data-demo="rui"]');
    if (!root) return;
    var QS = [
      { q: "价格对齐按哪个基准？",
        options: [
          { label: "卡片左边缘", desc: "价格与标题、描述左端对齐，整列成一条线" },
          { label: "图片左边缘", desc: "价格跟随主图起点，与卡片留白错开" }
        ] },
      { q: "窄屏下卡片怎么排？",
        options: [
          { label: "一行两张", desc: "图片缩小，信息保持完整" },
          { label: "一行一张", desc: "图片保持大小，滚动更长" }
        ] }
    ];
    var qi = 0, picks = {}, notes = {}, sent = false;
    var countEl = root.querySelector(".rui-count"), bodyEl = root.querySelector(".rui-body");
    var noteEl = root.querySelector(".rui-n"), submit = root.querySelector('[data-act="submit"]');

    function draw() {
      if (sent) {
        countEl.textContent = "";
        bodyEl.innerHTML = QS.map(function (q, i) {
          return '<div class="rui-done"><b>' + q.q + "</b><span>" + (picks[i] != null ? q.options[picks[i]].label : "未答") +
            (notes[i] ? "　补充：" + notes[i] : "") + "</span></div>";
        }).join("");
        noteEl.hidden = true;
        submit.outerHTML = '<span class="pbtn ok" style="pointer-events:none">已交回</span>';
        return;
      }
      var q = QS[qi];
      countEl.textContent = "第 " + (qi + 1) + " 个，共 " + QS.length + " 个";
      bodyEl.innerHTML = '<div class="rui-q">' + q.q + "</div>" +
        q.options.map(function (o, i) {
          return '<button type="button" class="rui-o' + (picks[qi] === i ? " sel" : "") + '" data-i="' + i + '">' +
            "<b>" + o.label + "</b><span>" + o.desc + "</span></button>";
        }).join("");
      bodyEl.querySelectorAll(".rui-o").forEach(function (b) {
        b.addEventListener("click", function () {
          picks[qi] = Number(b.getAttribute("data-i"));
          if (qi < QS.length - 1) { notes[qi] = noteEl.value.trim(); noteEl.value = ""; qi += 1; }
          draw();
        });
      });
      noteEl.value = notes[qi] || "";
      var answered = Object.keys(picks).length === QS.length;
      submit.disabled = !answered;
      submit.textContent = answered ? "提交" : "答完全部才能提交";
    }
    submit.addEventListener("click", function () { notes[qi] = noteEl.value.trim(); sent = true; draw(); });
    draw();
  })();

  /* —— 候选 C：逐题审批卡（Beautiful UI ApprovalCard 的结构） —— */
  (function approvalCard() {
    var root = document.querySelector('[data-demo="ap"]');
    if (!root) return;
    var QS = [
      { q: "价格对齐按哪个基准？", type: "radio", options: ["卡片左边缘", "图片左边缘", "先不动"] },
      { q: "窄屏下哪些信息可以先收起？", type: "check", options: ["库存标签", "促销角标", "评分"] },
      { q: "这一轮先改哪一块？", type: "radio", options: ["商品卡", "列表页栅格", "两块一起"] }
    ];
    var qi = 0, answers = {}, sent = false, timer = null;
    var view = root.querySelector(".ap-view"), stack = root.querySelector(".ap-stack");
    var count = root.querySelector(".ap-count"), foot = root.querySelector(".ap-foot");
    var pad = root.querySelector(".ap-pad");

    function heights() {
      var items = stack.querySelectorAll(".ap-q");
      return [].map.call(items, function (el) { return el.offsetHeight; });
    }
    function layout(animate) {
      var hs = heights();
      view.style.transition = animate ? "height 360ms " + EASE : "none";
      view.style.height = (hs[qi] || 0) + "px";
      stack.style.transition = animate ? "transform 360ms " + EASE : "none";
      var offset = hs.slice(0, qi).reduce(function (a, b) { return a + b; }, 0);
      stack.style.transform = "translateY(" + -offset + "px)";
      count.textContent = (qi + 1) + " / " + QS.length;
    }
    function draw(animate) {
      if (sent) {
        pad.innerHTML = '<div class="ap-sent"><span class="chip"><span class="tick"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round"><path d="M20 6L9 17l-5-5"/></svg></span>答案已交回</span>' +
          '<button type="button" class="ap-again">重来一次</button></div>';
        foot.hidden = true;
        pad.querySelector(".ap-again").addEventListener("click", function () { location.reload(); });
        return;
      }
      stack.innerHTML = QS.map(function (q, i) {
        var picked = answers[i] || [];
        return '<div class="ap-q"><div class="ap-qt">' + q.q + "</div>" +
          q.options.map(function (o, oi) {
            var on = picked.indexOf(oi) >= 0;
            return '<button type="button" class="ap-o' + (on ? " on" : "") + '" data-q="' + i + '" data-o="' + oi + '">' +
              '<span class="mark ' + q.type + '"></span>' + o + "</button>";
          }).join("") +
          '<div class="ap-other">其他想法…</div></div>';
      }).join("");
      stack.querySelectorAll(".ap-o").forEach(function (b) {
        b.addEventListener("click", function () {
          var i = Number(b.getAttribute("data-q")), oi = Number(b.getAttribute("data-o"));
          var cur = answers[i] || [];
          if (QS[i].type === "radio") answers[i] = [oi];
          else answers[i] = cur.indexOf(oi) >= 0 ? cur.filter(function (x) { return x !== oi; }) : cur.concat([oi]);
          draw(true);
          if (QS[i].type === "radio") {
            clearTimeout(timer);
            timer = setTimeout(function () { advance(); }, 260);
          }
        });
      });
      layout(animate);
    }
    function advance() { if (qi >= QS.length - 1) { sent = true; draw(false); } else { qi += 1; draw(true); } }
    foot.addEventListener("click", function (e) {
      var act = e.target.closest("[data-act]");
      if (!act) return;
      var a = act.getAttribute("data-act");
      if (a === "prev") { qi = Math.max(0, qi - 1); draw(true); }
      if (a === "next") { qi = Math.min(QS.length - 1, qi + 1); draw(true); }
      if (a === "skip") advance();
      if (a === "continue") advance();
    });
    root.querySelector(".ap-x").addEventListener("click", function () { root.classList.toggle("folded"); });
    draw(false);
    requestAnimationFrame(function () { layout(false); });
  })();
})();
