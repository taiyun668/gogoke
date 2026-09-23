(function () {
  var ICON = {
    run: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M12 3a9 9 0 1 0 9 9" stroke-linecap="round"/></svg>',
    done: '<svg width="15" height="15" viewBox="0 0 24 24" fill="currentColor"><circle cx="12" cy="12" r="10" opacity=".22"/><path d="M8 12.4l2.6 2.6L16.2 9.4" fill="none" stroke="currentColor" stroke-width="2.1" stroke-linecap="round"/></svg>',
    error: '<svg width="15" height="15" viewBox="0 0 24 24" fill="currentColor"><circle cx="12" cy="12" r="10" opacity=".22"/><path d="M9 9l6 6M15 9l-6 6" fill="none" stroke="currentColor" stroke-width="2.1" stroke-linecap="round"/></svg>',
    held: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M5 12h8M13 8l4 4-4 4" stroke-linecap="round" stroke-linejoin="round"/></svg>',
    unknown: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="9" stroke-dasharray="3 3"/></svg>'
  };
  var CHEV = '<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M9 6l6 6-6 6"/></svg>';
  var CARET = '<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M6 9l6 6 6-6"/></svg>';
  var BOT = '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"><rect x="4" y="8" width="16" height="12" rx="3"/><path d="M12 4v4M9 14h.01M15 14h.01"/></svg>';
  var ALERT = '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M12 8v5M12 16.5v.5"/><circle cx="12" cy="12" r="9"/></svg>';
  var WORD = { run: "运行中", done: "已收口", error: "失败", held: "等主控处理", unknown: "待核对" };

  var stream = document.getElementById("stream");
  var slot = document.getElementById("composer-slot");
  var nowLine = document.getElementById("now");
  var clockEl = document.getElementById("clock");

  var tasks = [];           // {id,name,status,task,log:[],report,startedAt,endedAt,note}
  var selected = null;
  var net = null, nodesEl = null, graphEl = null, rootPill = null, panelEl = null, composerEl = null, typingNode = null;
  var elapsed = 0, ticker = null, speed = 1, playing = true, waitingForYou = false;
  var queue = [], qi = 0, timer = null;

  function el(tag, cls, html) {
    var n = document.createElement(tag);
    if (cls) n.className = cls;
    if (html != null) n.innerHTML = html;
    return n;
  }
  function label(sec) {
    if (sec < 60) return sec + "s";
    var m = Math.floor(sec / 60), s = sec % 60;
    return s ? m + "m " + s + "s" : m + "m";
  }
  function say(t) { nowLine.textContent = t; }
  function get(id) { for (var i = 0; i < tasks.length; i++) if (tasks[i].id === id) return tasks[i]; return null; }
  function above(node) {
    if (net && net.parentNode === stream) stream.insertBefore(node, net);
    else stream.appendChild(node);
    return node;
  }

  /* ---- 一条失败拆成「出了什么事」和「怎么办」 ---- */
  function failure(text) {
    var m = /^([\s\S]*?[。．.;；!！?？])\s*([\s\S]+)$/.exec(String(text || "").trim());
    if (!m) return { what: text, fix: null };
    return { what: m[1], fix: m[2] };
  }
  /* ---- 报告的第一句有内容的话，就是这一行要显示的结论 ---- */
  function gist(report) {
    if (!report) return null;
    var lines = report.split("\n");
    for (var i = 0; i < lines.length; i++) {
      var line = lines[i].replace(/^[-*+]\s+/, "").trim();
      if (line) return line;
    }
    return null;
  }
  /* ---- 工具计数：读了几次、改了几次 ---- */
  function tally(log) {
    var order = [], counts = {};
    log.forEach(function (item) {
      if (item.kind !== "tool") return;
      if (!(item.verb in counts)) { counts[item.verb] = 0; order.push(item.verb); }
      counts[item.verb] += 1;
    });
    return order.map(function (v) { return { verb: v, count: counts[v], write: v === "改" || v === "写" || v === "提交" }; });
  }
  function actionCount(t) { return t.log.filter(function (i) { return i.kind === "tool"; }).length; }

  /* ---------- 编组卡 ---------- */
  function makeNet() {
    net = el("div", "net open enter");
    net.innerHTML =
      '<button class="net-head" type="button">' +
        '<span class="net-title">' + BOT + "正在进行</span>" +
        '<span class="net-counts"></span>' +
        '<span class="net-summary"></span>' +
        '<span class="net-state net-caret">' + CARET + "</span>" +
      "</button>" +
      '<div class="net-progress"><i></i></div>' +
      '<div class="disclose on net-body"><div><div class="graph">' +
        '<div class="graph-root"><span class="root-pill" data-running="true">' + BOT + " 主控</span></div>" +
        '<svg class="wires" aria-hidden></svg>' +
        '<div class="nodes"></div>' +
      "</div></div></div>" +
      '<div class="disclose panel-wrap"><div></div></div>' +
      '<div class="working-slot"></div>';
    net.querySelector(".net-head").addEventListener("click", function () {
      net.classList.toggle("open");
      net.querySelector(".net-body").classList.toggle("on", net.classList.contains("open"));
      if (!net.classList.contains("open")) net.querySelector(".panel-wrap").classList.remove("on");
      else if (selected) net.querySelector(".panel-wrap").classList.add("on");
      requestAnimationFrame(wires);
    });
    nodesEl = net.querySelector(".nodes");
    graphEl = net.querySelector(".graph");
    rootPill = net.querySelector(".root-pill");
    stream.appendChild(net);
    render();
  }

  function addTask(id, name, task) {
    tasks.push({ id: id, name: name, status: "run", task: task, log: [], report: null, startedAt: elapsed, endedAt: null, note: "已派发，正在启动" });
    render();
  }
  function act(id, verb, detail) {
    var t = get(id); if (!t) return;
    t.log.push({ kind: "tool", verb: verb, detail: detail || "" });
    t.note = null;
    render();
  }
  function note(id, text) {
    var t = get(id); if (!t) return;
    t.log.push({ kind: "note", text: text });
    t.note = text;
    render();
  }
  function status(id, s, extra) {
    var t = get(id); if (!t) return;
    extra = extra || {};
    t.status = s;
    if (extra.note !== undefined) t.note = extra.note;
    if (extra.report) t.report = extra.report;
    if (extra.error) t.error = extra.error;
    if (s !== "run") t.endedAt = elapsed;
    render();
  }

  /* ---------- 渲染 ---------- */
  function render() {
    if (!net) return;
    var run = 0, done = 0, failed = 0, held = 0, unknown = 0, actions = 0, settled = 0;
    tasks.forEach(function (t) {
      actions += actionCount(t);
      if (t.status === "run") run++;
      else if (t.status === "done") { done++; settled++; }
      else if (t.status === "error") { failed++; settled++; }
      else if (t.status === "held") held++;
      else unknown++;
    });
    var counts = [];
    if (run) counts.push(run + " 处理中");
    if (done) counts.push(done + " 改动就绪");
    if (failed) counts.push(failed + " 失败");
    if (held) counts.push(held + " 等主控处理");
    if (unknown) counts.push(unknown + " 待核对");
    net.querySelector(".net-counts").textContent = counts.join(" · ");
    net.querySelector(".net-summary").textContent =
      (actions ? actions + " 动作 · " : "") + label(elapsed);
    var fill = net.querySelector(".net-progress i");
    fill.style.transform = "scaleX(" + (tasks.length ? settled / tasks.length : 0) + ")";
    net.querySelector(".net-progress").setAttribute("aria-label", settled + " / " + tasks.length + " 个席位已收口");
    rootPill.setAttribute("data-running", String(run > 0));

    nodesEl.innerHTML = "";
    tasks.forEach(function (t) {
      var live = t.status === "run" ? t.log[t.log.length - 1] : null;
      var line;
      if (live && live.kind === "tool") {
        line = '<span class="node-verb">' + live.verb + "</span>" +
               (live.detail ? '<span class="node-detail">' + live.detail + "</span>" : "");
      } else if (t.status === "error" && t.error) {
        line = '<span class="node-say bad">' + failure(t.error).what + "</span>";
      } else if (t.status === "done" && t.report) {
        line = '<span class="node-say">' + gist(t.report) + "</span>";
      } else {
        line = '<span class="node-say' + (t.status === "held" ? "" : "") + '">' + (t.note || (live && live.text) || t.task) + "</span>";
      }
      var node = el("button", "node");
      node.type = "button";
      node.setAttribute("data-selected", String(selected === t.id));
      node.setAttribute("aria-expanded", String(selected === t.id));
      node.setAttribute("title", t.task);
      node.setAttribute("data-status", t.status);
      node.innerHTML =
        '<span class="avatar">' + ICON[t.status] + "</span>" +
        '<span class="node-name">' + t.name + "</span>" +
        '<span class="node-line">' + line + "</span>" +
        '<span class="node-state">' + CHEV + "</span>";
      node.addEventListener("click", function () {
        selected = selected === t.id ? null : t.id;
        render();
        requestAnimationFrame(wires);
      });
      nodesEl.appendChild(node);
    });

    // 详情面板：整张图下面，一次只开一个
    var wrap = net.querySelector(".panel-wrap");
    var host = wrap.firstElementChild;
    var sel = selected ? get(selected) : null;
    wrap.classList.toggle("on", Boolean(sel) && net.classList.contains("open"));
    if (!sel) { host.innerHTML = ""; panelEl = null; }
    if (sel) {
      var steps = tally(sel.log);
      var time = label((sel.endedAt == null ? elapsed : sel.endedAt) - sel.startedAt);
      panelEl = el("div", "panel");
      panelEl.innerHTML =
        '<div class="panel-head">' +
          '<span class="panel-title">' + sel.name + " <em>" + WORD[sel.status] + "</em></span>" +
          '<span class="steps">' + steps.map(function (s) {
            return '<span class="step" data-write="' + s.write + '">' + s.verb + " " + s.count + "</span>";
          }).join("") + "</span>" +
          '<span class="panel-time">' + time + "</span>" +
        "</div>" +
        '<p class="panel-ask">' + sel.task + "</p>" +
        (sel.status === "error" && sel.error ? failHtml(sel.error) : "") +
        '<div class="log">' + sel.log.map(function (i) {
          return i.kind === "tool"
            ? '<div class="log-line"><span class="log-verb">' + i.verb + "</span>" +
              (i.detail ? '<span class="log-detail">' + i.detail + "</span>" : "") + "</div>"
            : '<div class="log-line log-note">' + i.text + "</div>";
        }).join("") + "</div>" +
        (sel.report ? '<div class="report">' + sel.report.split("\n").map(function (l) { return "<div>" + l + "</div>"; }).join("") + "</div>" : "");
      host.innerHTML = "";
      host.appendChild(panelEl);
    }
    var slotEl = net.querySelector(".working-slot");
    var runner = null;
    for (var i = 0; i < tasks.length; i++) if (tasks[i].status === "run") { runner = tasks[i]; break; }
    if (runner && net.classList.contains("open")) {
      var last = runner.log[runner.log.length - 1];
      var what = last && last.kind === "tool" ? runner.name + " 正在" + last.verb + " " + (last.detail || "") : runner.name + " 正在处理";
      slotEl.innerHTML = '<div class="working"><span class="working-spinner"></span>' +
        '<span class="working-text">' + what + "</span>" +
        '<span class="working-timer">' + label(elapsed - runner.startedAt) + "</span></div>";
    } else {
      slotEl.innerHTML = "";
    }
    requestAnimationFrame(wires);
  }

  function failHtml(error) {
    var f = failure(error);
    return '<p class="fail">' + ALERT + '<span>' + f.what + (f.fix ? '<span class="fail-fix">' + f.fix + "</span>" : "") + "</span></p>";
  }

  /* ---------- 连线：量出来的，不是算出来的 ---------- */
  function wires() {
    if (!net || !net.classList.contains("open") || !graphEl) return;
    var svg = net.querySelector(".wires");
    var box = graphEl.getBoundingClientRect(), from = rootPill.getBoundingClientRect();
    if (!box.width) return;
    svg.setAttribute("viewBox", "0 0 " + box.width + " " + box.height);
    svg.style.width = box.width + "px"; svg.style.height = box.height + "px";
    var x1 = from.right - box.left, y1 = from.top + from.height / 2 - box.top, out = "";
    Array.prototype.forEach.call(nodesEl.children, function (node, i) {
      var t = tasks[i]; if (!t) return;
      var to = node.getBoundingClientRect();
      var x2 = to.left - box.left + 2, y2 = to.top + to.height / 2 - box.top;
      var bend = Math.max((x2 - x1) / 2, 8);
      var d = "M " + x1 + " " + y1 + " C " + (x1 + bend) + " " + y1 + ", " + (x2 - bend) + " " + y2 + ", " + x2 + " " + y2;
      out += '<g><path class="wire" d="' + d + '"></path>' +
             (t.status === "run" ? '<path class="wire-flow" d="' + d + '"></path>' : "") +
             '<circle class="wire-end" cx="' + x2 + '" cy="' + y2 + '" r="2"></circle></g>';
    });
    if (tasks.length) out += '<circle class="wire-port" cx="' + x1 + '" cy="' + y1 + '" r="2.5"></circle>';
    svg.innerHTML = out;
  }

  /* ---------- 主控的输出 ---------- */
  function userMsg(t) { return above(el("div", "msg user enter", '<div class="body">' + t + "</div>")); }
  function botMsg(t) { return above(el("div", "msg enter", '<div class="whoami">主控</div><div class="body">' + t + "</div>")); }
  function typing() { return above(el("div", "msg enter", '<div class="whoami">主控</div><div class="body dots" style="color:var(--text-dim)"><span>*</span><span>*</span><span>*</span></div>')); }
  function composer(left, right) {
    if (!composerEl) { composerEl = el("div", "composer"); slot.appendChild(composerEl); }
    composerEl.innerHTML = '<span class="to">' + left + "</span><span>" + (right || "") + "</span>";
  }

  function controllerAsks() {
    waitingForYou = true; pause(true);
    say("停在这里等你。审计把问题交给主控，主控自己定不了，才来问你 —— 卡挂在主控名下。");

    /* 选项形状照 RecommendationCard 的 RecommendationOption：body / short / signal / tone / label / cta */
    var OPTIONS = [
      { key: "card",
        body: '按 <span class="ent"><span class="mono" style="background:#5b8def">规</span><span class="nm">项目视觉规范</span></span> 的写法，价格与标题左端对齐，影响 <span class="vp green">3 个文件</span>',
        short: "卡片左边缘 · 规范里写过", signal: 3, tone: "var(--status-success)", label: "有明确依据", cta: "就按这个", ctaVariant: "accent" },
      { key: "img",
        body: '价格跟随主图起点，与卡片留白错开。规范里没写过这一种，<span class="vp neutral">需要你定</span>',
        short: "图片左边缘", signal: 2, tone: "var(--status-warning)", label: "需要你定", cta: "改用这个", ctaVariant: "primary" },
      { key: "keep",
        body: '这一轮先不动对齐，只改行距，价格位置留到下一轮再说。',
        short: "先不动对齐", signal: 0, tone: "var(--status-unknown)", label: "没有依据", cta: "先不动", ctaVariant: "primary" }
    ];
    var selected = 0, open = false, accepted = false;

    var card = el("div", "msg pop");
    card.innerHTML = '<div class="whoami">主控</div><div class="rc"></div>';
    var box = card.querySelector(".rc");
    above(card);

    function meter(signal, tone) {
      var out = '<span class="meter">';
      for (var i = 0; i < 3; i++) out += '<i style="background:' + (i < signal ? tone : "var(--border-strong)") + '"></i>';
      return out + "</span>";
    }

    function draw() {
      var a = OPTIONS[selected];
      var others = OPTIONS.map(function (o, i) { return { o: o, i: i }; }).filter(function (x) { return x.i !== selected; });
      box.innerHTML =
        '<div class="rc-pad">' +
          '<span class="rc-title">价格对齐按哪个基准？</span>' +
          '<p class="rc-body">' + a.body + "</p>" +
        "</div>" +
        '<div class="rc-drawer' + (open ? " on" : "") + '"><div><div class="rc-drawer-inner">' +
          '<p class="rc-drawer-label">其他选项</p>' +
          others.map(function (x) {
            return '<button type="button" class="rc-alt" data-i="' + x.i + '">' + meter(x.o.signal, x.o.tone) +
              '<span class="rc-alt-short">' + x.o.short + "</span>" +
              '<span class="rc-alt-label">' + x.o.label + "</span></button>";
          }).join("") +
        "</div></div></div>" +
        '<div class="rc-foot">' +
          '<span class="lvl">' + meter(a.signal, a.tone) + a.label + "</span>" +
          '<span class="acts">' +
            '<button type="button" class="btn secondary alts" aria-expanded="' + open + '">其他选项</button>' +
            '<button type="button" class="btn ' + (accepted ? "success" : a.ctaVariant) + ' take"' + (accepted ? " disabled" : "") + ">" +
              (accepted ? "已采纳" : a.cta) + "</button>" +
          "</span>" +
        "</div>";

      box.querySelector(".alts").addEventListener("click", function () { open = !open; draw(); });
      box.querySelectorAll(".rc-alt").forEach(function (b) {
        b.addEventListener("click", function () { selected = Number(b.getAttribute("data-i")); accepted = false; draw(); });
      });
      box.querySelector(".take").addEventListener("click", function () {
        accepted = true; open = false; draw();
        status("audit", "run", { note: null });
        act("audit", "读", "src/card.css");
        waitingForYou = false; pause(false); next();
      });
    }
    draw();
  }

  function gate() {
    waitingForYou = true; pause(true);
    say("停在这里等你。段落对账点是房间的闸门，自动接力已暂停。");
    var g = el("div", "card gate pop");
    g.innerHTML =
      '<div class="gt">到达段落对账点</div>' +
      '<div class="gb">自动接力已暂停。目标锚点：商品卡信息层级清楚，价格逻辑不变。checkpoint <code>8f21a4</code> → HEAD <code>e4f5a67</code>，净改动 3 个文件。</div>' +
      '<div class="ga"><button type="button" data-g="on">交给审计核对</button><button type="button" data-g="stop">先停下</button></div>';
    above(g);
    g.querySelectorAll("button").forEach(function (b) {
      b.addEventListener("click", function () {
        if (b.getAttribute("data-g") === "stop") {
          g.innerHTML = '<div class="gt">已请求停止</div><div class="gb">在途回合可以收尾，不会再发起新接力。已产生的改动不会自动回退。</div>' +
            '<div class="ga"><button type="button">还是继续</button></div>';
          g.querySelector("button").addEventListener("click", function () {
            g.querySelector(".ga").remove(); waitingForYou = false; pause(false); next();
          });
          return;
        }
        g.querySelector(".ga").remove();
        g.querySelector(".gb").innerHTML += "　你选了继续。";
        waitingForYou = false; pause(false); next();
      });
    });
  }

  function finale() {
    net.classList.remove("open");
    net.querySelector(".net-title").innerHTML = BOT + "商品卡太挤，改清楚";
    net.querySelector(".net-counts").innerHTML = '<span class="state done">可验收</span> · 对账已过，独立性 PASS，verify ok';
    var accept = el("div", "msg pop");
    accept.innerHTML = '<div class="whoami">主控</div><div class="card accept">' +
      '<div class="gt">这一段做完了，等你验收</div>' +
      '<div class="gb">成果：商品卡排版，改了 3 个文件<span style="color:var(--text-dim)"> · 版本 <code>e4f5a67</code></span>。' +
      '审计独立性 PASS（施工 codex / 审计 claude，provider family 不同）；verify ok 由房间记录，不采用审计席自述。</div>' +
      '<div class="ga"><button type="button">验收这件</button><button type="button">看改动</button><button type="button">还要再改</button></div></div>';
    above(accept);
    accept.querySelectorAll("button")[0].addEventListener("click", function () {
      accept.querySelector(".ga").remove();
      accept.querySelector(".gt").textContent = "已验收";
      accept.querySelector(".gb").innerHTML = "今天 " + new Date().toTimeString().slice(0, 5) + " 由你验收。这段目标不再接受派发；后续改动要新开目标。";
      net.querySelector(".net-counts").innerHTML = '<span class="state acc">已验收</span> · 版本 <code>e4f5a67</code>';
      say("走完了。这件事现在就是列表里的一行。点上面的重播再看一遍。");
    });
    say("归一：三条线都收回主控，正在进行折成一行，状态变成可验收。");
  }

  /* ---------- 时间线 ---------- */
  function script() {
    return [
      [300, function () { say("你把要求说给主控。"); userMsg("商品卡太挤了，帮我改清楚一点，价格别动。"); composer("发给 主控", ""); }],
      [900, function () { typingNode = typing(); }],
      [1500, function () {
        if (typingNode) { typingNode.remove(); typingNode = null; }
        botMsg("先让施工改卡片排版，审计核对视觉规范，秘书查这块历史上改过几次。价格逻辑我标成禁止修改。");
        say("主控在回复里 @ 了三个席位 —— 派活就是这一步。");
      }],
      [700, function () { makeNet(); say("最底下这块永远是正在进行；主控之后说的话都堆在它上面。"); }],
      [450, function () { addTask("build", "施工 Codex", "商品卡信息层级改清楚；价格逻辑禁止修改。"); say("发散：一条 @ 派给三个席位，就分出三条线。"); }],
      [380, function () { addTask("sec", "秘书 Grok", "查商品卡样式历史上改过几次、分别因为什么。只读。"); }],
      [380, function () { addTask("audit", "审计 Claude", "按视觉规范核对施工交回的版本，独立判断。"); }],
      [800, function () { act("build", "读", "src/card.tsx"); }],
      [650, function () { act("sec", "搜", "卡片 间距"); }],
      [650, function () { act("audit", "读", "docs/ui-rules.md"); }],
      [750, function () {
        note("sec", "问历史记录放在哪，已交给主控");
        status("sec", "held");
        say("秘书有问题了 —— 它问的是主控，不是你。");
      }],
      [900, function () {
        botMsg('秘书问历史记录在哪，我直接告诉它了：<code>docs/history/</code>，不用你管。');
        status("sec", "run", { note: null });
        act("sec", "读", "docs/history/card-changes.md");
        say("主控自己答得上来的就自己答了，你这边只看到一句说明。");
      }],
      [800, function () { act("build", "改", "src/card.css"); }],
      [850, function () {
        note("audit", "核对卡在价格基准，已交给主控");
        status("audit", "held");
        say("审计也有问题，同样先交给主控 —— 这次主控自己定不了。");
      }],
      [600, controllerAsks],
      [700, function () { act("build", "改", "src/grid.css"); }],
      [700, function () { act("sec", "读", "docs/history/2025-11-card.md"); }],
      [800, function () {
        status("sec", "unknown", { note: "投递结果待核对，不自动重发" });
        say("秘书那条没收到回执 —— 如实标成待核对，不计入任何一边。");
      }],
      [1000, function () {
        act("build", "提交", "a1b2c3d 商品卡：行距与价格位置");
        act("build", "提交", "e4f5a67 卡片栅格在窄屏换行");
        status("build", "done", { report:
          "商品卡行距和价格位置已改，价格逻辑一行未动。\n" +
          "commit a1b2c3d 商品卡：行距与价格位置\n" +
          "commit e4f5a67 卡片栅格在窄屏换行\n" +
          "涉及文件 src/card.tsx  src/card.css  src/grid.css\n" +
          "交给下一席位的是这些仓库引用，不是这段话" });
        above(el("div", "turn-complete enter", '<span class="turn-complete-label">施工这一轮结束</span><span class="turn-complete-line"></span>'));
        say("施工收口。它那一行现在显示的是它交回的第一句结论，不是状态词。");
      }],
      [900, gate],
      [700, function () { act("audit", "读", "e4f5a67 的净改动"); }],
      [1100, function () {
        act("audit", "跑", "npm test -- card");
        status("audit", "done", { report:
          "三个文件的改动与视觉规范一致，价格逻辑未被改动。\n" +
          "被审对象 e4f5a67，结论绑定这个版本\n" +
          "独立性由房间按 provider family 记录，不采用审计席自述" });
      }],
      [900, finale]
    ];
  }

  function next() {
    if (qi >= queue.length) return;
    var step = queue[qi++];
    timer = setTimeout(function () {
      step[1]();
      if (!waitingForYou) next();
    }, step[0] / speed);
  }
  function pause(on) {
    playing = !on;
    document.getElementById("play").textContent = playing ? "暂停" : "继续";
    if (on) { clearTimeout(timer); clearInterval(ticker); ticker = null; }
    else if (!ticker) ticker = setInterval(tick, 1000 / speed);
  }
  function tick() { elapsed += 1; clockEl.textContent = label(elapsed); if (net) render(); }

  function start() {
    clearTimeout(timer); clearInterval(ticker); ticker = null;
    stream.innerHTML = ""; slot.innerHTML = "";
    tasks = []; selected = null;
    net = nodesEl = graphEl = rootPill = panelEl = composerEl = typingNode = null;
    elapsed = 0; clockEl.textContent = "0s"; waitingForYou = false;
    queue = script(); qi = 0; playing = true;
    document.getElementById("play").textContent = "暂停";
    ticker = setInterval(tick, 1000 / speed);
    next();
  }

  document.getElementById("play").addEventListener("click", function () {
    if (waitingForYou) { say("先在上面点一下，流程才会继续。"); return; }
    pause(playing);
    if (playing) next();
  });
  document.getElementById("restart").addEventListener("click", start);
  document.querySelectorAll(".spd").forEach(function (b) {
    b.addEventListener("click", function () {
      speed = Number(b.getAttribute("data-s"));
      document.querySelectorAll(".spd").forEach(function (x) { x.setAttribute("aria-pressed", String(x === b)); });
      if (ticker) { clearInterval(ticker); ticker = setInterval(tick, 1000 / speed); }
    });
  });
  window.addEventListener("resize", wires);
  start();
})();
