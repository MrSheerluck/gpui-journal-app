

const MESSAGES = {
  joined: {
    cls: "text-primary",
    text: "You're on the list. We'll be in touch soon.",
  },
  duplicate: {
    cls: "text-on-surface-variant",
    text: "You're already on the waitlist.",
  },
  invalid: {
    cls: "text-error",
    text: "Please enter a valid email address.",
  },
  error: {
    cls: "text-error",
    text: "Something went wrong. Please try again in a moment.",
  },
};

const STATUS_PLACEHOLDER = "<!--WAITLIST_STATUS-->";

export default {
  async fetch(request, env) {
    const url = new URL(request.url);

    if (url.pathname === "/waitlist") {
      if (request.method !== "POST") {
        return new Response("Method Not Allowed", {
          status: 405,
          headers: { allow: "POST" },
        });
      }
      return submitWaitlist(request, env, url);
    }

    
    
    
    const code = url.searchParams.get("joined");
    if (url.pathname === "/" && code && MESSAGES[code]) {
      const asset = await env.ASSETS.fetch(
        new Request(new URL("/", url.origin), request),
      );
      const html = (await asset.text()).replace(
        STATUS_PLACEHOLDER,
        statusHtml(MESSAGES[code]),
      );
      return new Response(html, {
        headers: {
          "content-type": "text/html;charset=UTF-8",
          "cache-control": "no-store",
        },
      });
    }

    return env.ASSETS.fetch(request);
  },
};

function statusHtml({ cls, text }) {
  return `<p class="mt-4 font-ui-caption text-ui-caption max-w-lg ${cls}" role="status">${escapeHtml(text)}</p>`;
}

async function submitWaitlist(request, env, url) {
  let form;
  try {
    form = await request.formData();
  } catch {
    return redirect(url, "error");
  }

  const email = String(form.get("email") ?? "").trim().toLowerCase();
  const honeypot = String(form.get("website") ?? "").trim();

  let code;
  if (honeypot) {
    
    code = "joined";
  } else if (!isValidEmail(email)) {
    code = "invalid";
  } else {
    try {
      const result = await env.WAITLIST.prepare(
        "INSERT INTO waitlist (email) VALUES (?) ON CONFLICT(email) DO NOTHING",
      )
        .bind(email)
        .run();
      code = result.meta?.changes ? "joined" : "duplicate";
    } catch (error) {
      console.error("wishlist insert failed:", error);
      code = "error";
    }
  }

  return redirect(url, code);
}

function redirect(url, code) {
  return new Response(null, {
    status: 303,
    headers: { location: `/?joined=${code}` },
  });
}


function isValidEmail(email) {
  if (email.length < 5 || email.length > 254) return false;
  const parts = email.split("@");
  if (parts.length !== 2) return false;
  const [local, domain] = parts;
  if (!local || !/^[A-Za-z0-9._%+-]+$/.test(local)) return false;
  if (domain.length > 253 || !/^[A-Za-z0-9.-]+$/.test(domain)) return false;
  const labels = domain.split(".");
  const tld = labels.pop();
  if (labels.length < 1) return false; 
  if (!tld || tld.length < 2 || !/^[A-Za-z]+$/.test(tld)) return false;
  return labels.every((label) => label.length > 0);
}

function escapeHtml(text) {
  return text
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;");
}
