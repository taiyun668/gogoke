// Run as one mcp__cua_repl.js call after the tool's first-use initialization.
// prepare_dispatch.py replaces the placeholder with a JSON task payload.
{
  const cfg = __CONFIG__;
  const labels = {
    "gpt-5.6-sol-pro": { item: 5, text: "5.6 Pro" },
    "extra-high": { item: 4, text: "5.6 极高" },
    high: { item: 3, text: "5.6 高" },
    medium: { item: 2, text: "5.6 中" },
  };
  let tab;
  let clicked = false;
  let result = { task: cfg.task, tier: cfg.tier, status: "not_sent", clicked: false };
  const fullAX = async () => await tab.getAXState({ disableDiffing: true, emit: false });
  const indexOf = (ax, pattern, name) => {
    const match = ax.match(pattern);
    if (!match) throw new Error(`${name} not found in current accessibility state`);
    return Number(match[1]);
  };
  try {
    if (!labels[cfg.tier]) throw new Error("GPT-6 Pro and unknown tiers are forbidden by this trial browser script");
    const before = await cua.getState({ emit: false });
    const iab = before.browsers.find((b) => b.type === "iab");
    if (!iab) throw new Error("Codex in-app browser unavailable");
    const chatTabs = iab.tabs.filter((t) => /^https:\/\/chatgpt\.com(?:\/|$)/.test(t.url || ""));
    if (chatTabs.length !== 0) throw new Error(`ChatGPT tab count ${chatTabs.length}; finish and close it before dispatch`);
    tab = await cua.createBrowserTab("iab", "https://chatgpt.com/", { visible: false });
    let ax = await fullAX();
    if (/登录|注册|验证码|安全验证|可疑活动|Log in|Sign up|Verify|suspicious activity/i.test(ax) && !/与 ChatGPT 聊天/.test(ax)) {
      result.status = /安全验证|可疑活动|Verify|suspicious activity/i.test(ax) ? "security_verification" : "needs_login";
      result.reason = "authentication_or_security_page";
      await tab.markHandoff();
    } else {
      await tab.playwright.locator("#prompt-textarea").waitFor({ state: "visible", timeoutMs: 15000 });
      ax = await fullAX();
      if (!/radio button 聊天, Value: 1/.test(ax) || /chatgpt\.com\/g\//.test(ax)) throw new Error("not a fresh saved Chat conversation outside Projects");
      const pill = indexOf(ax, /^\s*(\d+) pop up button \(collapsed\) (?:5\.6\s*)?(?:中|高|极高|Pro), ID: radix-/m, "model pill");
      await tab.pressKey(pill, "Return");
      ax = await fullAX();
      const modelSelector = indexOf(ax, /^\s*(\d+) \(collapsed\) Description: 选择模型/m, "model selector");
      await tab.pressKey(modelSelector, "Return");
      ax = await fullAX();
      const sol = ax.match(/^\s*(\d+) GPT-5\.6 Sol, Value: ([01])/m);
      if (!sol) throw new Error("GPT-5.6 Sol option unavailable");
      if (sol[2] !== "1") {
        await tab.pressKey(Number(sol[1]), "Return");
        ax = await fullAX();
      } else {
        await tab.pressKey(Number(sol[1]), "Escape");
        ax = await fullAX();
      }
      let ability = indexOf(ax, /^\s*(\d+) 能力\s*$/m, "thinking slider");
      let current = ax.match(/第 (\d+) 项，共 5 项/);
      if (!current) throw new Error("thinking level not observable");
      let level = Number(current[1]);
      while (level !== labels[cfg.tier].item) {
        await tab.pressKey(ability, level < labels[cfg.tier].item ? "Right" : "Left");
        ax = await fullAX();
        ability = indexOf(ax, /^\s*(\d+) 能力\s*$/m, "thinking slider");
        current = ax.match(/第 (\d+) 项，共 5 项/);
        if (!current || Number(current[1]) === level) throw new Error("thinking slider did not change");
        level = Number(current[1]);
      }
      await tab.pressKey(ability, "Escape");
      ax = await fullAX();
      if (ax.includes("6 Pro") && !ax.includes("5.6 Pro")) throw new Error("GPT-6 Pro selected; dispatch forbidden");
      const selected = new RegExp(`pop up button \\(collapsed\\) ${labels[cfg.tier].text.replace(".", "\\.")}, ID: radix-`);
      if (!selected.test(ax)) throw new Error(`selected composer tier differs from ${labels[cfg.tier].text}`);
      const composer = indexOf(ax, /^\s*(\d+) text entry area \(settable\) Description: 与 ChatGPT 聊天, ID: prompt-textarea/m, "composer");
      await tab.paste(composer, cfg.starter, { format: "text" });
      ax = await fullAX();
      const staged = await tab.playwright.locator("#prompt-textarea").innerText();
      if (staged.trim() !== cfg.starter.trim()) throw new Error("staged prompt differs from committed task routing fields");
      const justBeforeSend = await cua.getState({ emit: false });
      const currentIab = justBeforeSend.browsers.find((b) => b.type === "iab");
      const currentChatTabs = currentIab.tabs.filter((t) => /^https:\/\/chatgpt\.com(?:\/|$)/.test(t.url || ""));
      if (currentChatTabs.length !== 1 || currentChatTabs[0].id !== tab.id) throw new Error("ChatGPT tab count or identity changed before Send");
      if (/可疑活动|安全验证|suspicious activity|security verification/i.test(ax)) throw new Error("SECURITY_PAGE: warning before Send");
      if (!selected.test(ax) || /限额|usage limit/i.test(ax)) throw new Error("model changed or limit notice appeared before Send");
      const send = indexOf(ax, /^\s*(\d+) button Description: 发送提示词, ID: composer-submit-button/m, "Send button");
      clicked = true;
      await tab.pressKey(send, "Return");
      ax = await fullAX();
      if (/可疑活动|安全验证|suspicious activity|security verification/i.test(ax)) throw new Error("SECURITY_PAGE: warning after Send");
      if (/达到.*限额|已达.*上限|usage limit|rate limit/i.test(ax)) throw new Error("LIMIT_NO_REPLY: limit notice after Send");
      await tab.playwright.locator('[data-message-author-role="assistant"]').waitFor({ state: "attached", timeoutMs: 20000 });
      const userTurns = await tab.playwright.locator('[data-message-author-role="user"]').count();
      const assistantTurns = await tab.playwright.locator('[data-message-author-role="assistant"]').count();
      if (userTurns < 1 || assistantTurns < 1) throw new Error("new user and assistant turns not both observed");
      result = { task: cfg.task, tier: cfg.tier, status: "sent_confirmed", clicked: true, selected_model: labels[cfg.tier].text, conversation_url: await tab.url() };
      await tab.markHandoff();
    }
  } catch (error) {
    let latest = "";
    if (tab) {
      try { latest = await fullAX(); } catch (_) { /* keep the original failure */ }
    }
    const security = String(error).includes("SECURITY_PAGE") || /可疑活动|安全验证|suspicious activity|security verification/i.test(latest);
    let assistantCount = 0;
    if (tab && clicked) {
      try { assistantCount = await tab.playwright.locator('[data-message-author-role="assistant"]').count(); } catch (_) { /* unknown */ }
    }
    const limit = (String(error).includes("LIMIT_NO_REPLY") || clicked && /达到.*限额|已达.*上限|usage limit|rate limit/i.test(latest)) && assistantCount === 0;
    result = { task: cfg.task, tier: cfg.tier, status: security ? "security_verification" : limit ? "limit_no_reply" : clicked ? "uncertain_submission" : "not_sent", clicked, reason: String(error) };
    if (tab && (clicked || security)) {
      result.conversation_url = await tab.url();
      await tab.markHandoff();
    } else if (tab && result.status === "not_sent") {
      const lastAX = await fullAX();
      if (/登录|注册|验证码|安全验证|可疑活动|Log in|Sign up|Verify|suspicious activity/i.test(lastAX) && !/与 ChatGPT 聊天/.test(lastAX)) {
        result.status = /安全验证|可疑活动|Verify|suspicious activity/i.test(lastAX) ? "security_verification" : "needs_login";
        await tab.markHandoff();
      } else {
        await tab.close();
      }
    }
  }
  nodeRepl.write(JSON.stringify(result));
}
