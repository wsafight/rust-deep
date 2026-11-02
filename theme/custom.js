(() => {
    const article = document.querySelector("main");
    if (!article) return;

    const root = document.documentElement;
    root.classList.add("reader-enhanced");

    const controls = new Map([
        ["mdbook-sidebar-toggle", "打开或关闭章节目录"],
        ["mdbook-theme-toggle", "切换阅读主题"],
        ["mdbook-search-toggle", "搜索本书（/）"],
        ["mdbook-print-button", "打印本书"],
    ]);
    for (const [identifier, label] of controls) {
        const control = document.getElementById(identifier);
        if (!control) continue;
        control.title = label;
        control.setAttribute("aria-label", label);
    }
    const themeNames = new Map([
        ["default_theme", "跟随系统"], ["light", "浅色 · 纸页"],
        ["rust", "暖色 · Rust"], ["coal", "深色 · 煤黑"],
        ["navy", "深色 · 深海"], ["ayu", "深色 · Ayu"],
    ]);
    for (const [identifier, label] of themeNames) {
        const button = document.getElementById(`mdbook-theme-${identifier}`);
        if (button) button.textContent = label;
    }
    const search = document.getElementById("mdbook-searchbar");
    if (search) {
        search.placeholder = "搜索章节、概念或错误码…";
        search.setAttribute("aria-label", "搜索本书");
    }
    for (const button of document.querySelectorAll(".clip-button")) {
        button.title = "复制代码";
        button.setAttribute("aria-label", "复制代码");
    }

    const languageLabels = new Map([
        ["rust", "Rust"], ["bash", "Shell"], ["sh", "Shell"],
        ["shell", "Shell"], ["console", "终端输出"], ["text", "文本 / 输出"],
        ["asm", "汇编"], ["x86asm", "汇编"], ["aarch64", "AArch64 汇编"],
        ["llvm", "LLVM IR"], ["mir", "MIR"], ["c", "C"], ["toml", "TOML"],
    ]);
    for (const code of article.querySelectorAll("pre > code")) {
        const language = [...code.classList].find((name) => name.startsWith("language-"));
        const identifier = language?.slice(9) || "text";
        const label = languageLabels.get(identifier) || identifier;
        code.parentElement.dataset.language = label;
    }

    const sidebar = document.getElementById("mdbook-sidebar");
    const scrollbox = sidebar?.querySelector(".sidebar-scrollbox");
    if (scrollbox) {
        const brand = document.createElement("a");
        brand.className = "sidebar-brand";
        brand.href = `${typeof path_to_root === "string" ? path_to_root : "./"}index.html`;
        const name = document.createElement("strong");
        name.textContent = "Rust 深水区";
        const description = document.createElement("span");
        description.textContent = "从源码到证据，理解每一个为什么";
        brand.append(name, description);
        scrollbox.prepend(brand);
        sidebar.setAttribute("aria-label", "全书章节目录");
    }

    const chapterLinks = [...(scrollbox?.querySelectorAll(".chapter a[href]") || [])];
    for (const link of document.querySelectorAll(".nav-wrapper .mobile-nav-chapters")) {
        const direction = link.rel.split(" ").includes("prev") ? "上一章" : "下一章";
        const chapter = chapterLinks.find((entry) => entry.href === link.href);
        const label = document.createElement("span");
        label.className = "chapter-nav-label";
        const caption = document.createElement("small");
        caption.textContent = direction;
        const title = document.createElement("strong");
        title.textContent = chapter?.textContent.trim() || "继续阅读";
        label.append(caption, title);
        link.append(label);
        link.title = `${direction}：${title.textContent}`;
        link.setAttribute("aria-label", link.title);
    }

    const headings = [...article.querySelectorAll("h2[id], h3[id]")]
        .filter((heading) => !heading.closest("details"));
    if (headings.length > 1 && !location.pathname.endsWith("/print.html")) {
        const wideScreen = window.matchMedia("(min-width: 1400px)");
        const contents = document.createElement("details");
        contents.className = "page-toc";
        contents.open = wideScreen.matches;
        const summary = document.createElement("summary");
        summary.textContent = "本章目录";
        const navigation = document.createElement("nav");
        navigation.setAttribute("aria-label", "本章小节导航");
        const list = document.createElement("ol");
        const links = headings.map((heading) => {
            const item = document.createElement("li");
            if (heading.tagName === "H3") item.className = "toc-nested";
            const link = document.createElement("a");
            link.href = `#${encodeURIComponent(heading.id)}`;
            link.textContent = heading.textContent.trim();
            link.addEventListener("click", () => {
                if (!wideScreen.matches) contents.open = false;
                heading.tabIndex = -1;
                heading.focus({ preventScroll: true });
            });
            item.append(link);
            list.append(item);
            return link;
        });
        navigation.append(list);
        contents.append(summary, navigation);
        const title = article.querySelector("h1");
        if (title) title.after(contents);
        else article.prepend(contents);
        root.classList.add("has-page-toc");
        wideScreen.addEventListener("change", () => { contents.open = wideScreen.matches; });
        contents.addEventListener("toggle", () => {
            if (wideScreen.matches && !contents.open) contents.open = true;
        });

        let currentIndex = -1;
        let scheduled = false;
        const updateCurrentHeading = () => {
            scheduled = false;
            let activeIndex = 0;
            headings.forEach((heading, index) => {
                if (heading.getBoundingClientRect().top <= 112) activeIndex = index;
            });
            if (currentIndex === activeIndex) return;
            if (currentIndex >= 0) links[currentIndex].removeAttribute("aria-current");
            links[activeIndex].setAttribute("aria-current", "location");
            currentIndex = activeIndex;
        };
        const scheduleUpdate = () => {
            if (scheduled) return;
            scheduled = true;
            requestAnimationFrame(updateCurrentHeading);
        };
        window.addEventListener("scroll", scheduleUpdate, { passive: true });
        window.addEventListener("resize", scheduleUpdate, { passive: true });
        window.addEventListener("load", scheduleUpdate, { once: true });
        updateCurrentHeading();
    }

    const checkbox = document.getElementById("mdbook-sidebar-toggle-anchor");
    const toggle = document.getElementById("mdbook-sidebar-toggle");
    if (sidebar && checkbox && toggle) {
        const mobileScreen = window.matchMedia("(max-width: 619px)");
        const backdrop = document.createElement("button");
        backdrop.className = "sidebar-backdrop";
        backdrop.setAttribute("aria-label", "关闭章节目录");
        backdrop.tabIndex = -1;
        backdrop.hidden = true;
        const close = document.createElement("button");
        close.className = "sidebar-close";
        close.type = "button";
        close.textContent = "×";
        close.setAttribute("aria-label", "关闭章节目录");
        sidebar.append(close);
        sidebar.after(backdrop);
        toggle.tabIndex = 0;
        toggle.setAttribute("role", "button");

        const syncSidebar = () => {
            const opened = mobileScreen.matches && checkbox.checked;
            backdrop.hidden = !opened;
            root.classList.toggle("mobile-sidebar-open", opened);
            if (mobileScreen.matches) sidebar.inert = !opened;
            else sidebar.inert = false;
        };
        const closeSidebar = () => {
            checkbox.checked = false;
            checkbox.dispatchEvent(new Event("change", { bubbles: true }));
            toggle.focus();
        };
        close.addEventListener("click", closeSidebar);
        backdrop.addEventListener("click", closeSidebar);
        toggle.addEventListener("keydown", (event) => {
            if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                toggle.click();
            }
        });
        checkbox.addEventListener("change", () => {
            syncSidebar();
            if (mobileScreen.matches && checkbox.checked) close.focus();
        });
        mobileScreen.addEventListener("change", syncSidebar);
        document.addEventListener("keydown", (event) => {
            if (!mobileScreen.matches || !checkbox.checked) return;
            if (event.key === "Escape") {
                event.preventDefault();
                closeSidebar();
            }
            if (event.key === "Tab") {
                const focusable = [...sidebar.querySelectorAll("a[href], button")]
                    .filter((element) => element.getClientRects().length > 0);
                const first = focusable[0];
                const last = focusable[focusable.length - 1];
                if (event.shiftKey && document.activeElement === first) {
                    event.preventDefault();
                    last?.focus();
                } else if (!event.shiftKey && document.activeElement === last) {
                    event.preventDefault();
                    first?.focus();
                }
            }
        });
        syncSidebar();
    }
})();
