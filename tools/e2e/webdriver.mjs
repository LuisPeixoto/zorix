// A small WebDriver client, written for tauri-driver (ADR-0042): no dependency, only `fetch`. It speaks the W3C WebDriver protocol to the
// driver that starts the app, and offers what the scenarios need: find by XPath, click, type, read text, screenshot and page source.

export class Session {
  constructor(base, id) {
    this.base = base;
    this.id = id;
  }

  /** Start the app under the driver at `base` and open a session on it. */
  static async start(base, application) {
    const reply = await request(base, "POST", "/session", {
      capabilities: { alwaysMatch: { "tauri:options": { application } } },
    });
    return new Session(base, reply.value.sessionId);
  }

  async call(method, path, body) {
    return (await request(this.base, method, `/session/${this.id}${path}`, body)).value;
  }

  async find(xpath) {
    const found = await this.call("POST", "/elements", { using: "xpath", value: xpath });
    return found.map((element) => Object.values(element)[0]);
  }

  async click(id) {
    await this.call("POST", `/element/${id}/click`, {});
  }

  async text(id) {
    return this.call("GET", `/element/${id}/text`);
  }

  async type(id, text) {
    await this.call("POST", `/element/${id}/value`, { text });
  }

  /** Wait until `xpath` matches something (visible text not required), then give its first element. */
  async waitFor(xpath, { timeout = 30_000, label = xpath } = {}) {
    const until = Date.now() + timeout;
    let last = [];
    while (Date.now() < until) {
      last = await this.find(xpath).catch(() => []);
      if (last.length) return last[0];
      await sleep(250);
    }
    throw new Error(`Waited ${timeout / 1000} s for ${label}, and it never appeared.`);
  }

  async clickOn(xpath, options) {
    const element = await this.waitFor(xpath, options);
    await this.click(element);
  }

  async screenshot() {
    return Buffer.from(await this.call("GET", "/screenshot"), "base64");
  }

  async source() {
    return this.call("GET", "/source");
  }

  async end() {
    await request(this.base, "DELETE", `/session/${this.id}`).catch(() => undefined);
  }
}

export const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function request(base, method, path, body) {
  const response = await fetch(base + path, {
    method,
    headers: { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await response.text();
  let json;
  try {
    json = JSON.parse(text);
  } catch {
    throw new Error(`${method} ${path}: the driver answered ${response.status} with something that is not JSON: ${text.slice(0, 200)}`);
  }
  if (!response.ok) throw new Error(`${method} ${path}: ${json.value?.error ?? response.status}: ${json.value?.message ?? text.slice(0, 200)}`);
  return json;
}

/** XPath for an element whose own text, without the spaces around it, is `text`. */
export const withText = (tag, text) => `//${tag}[normalize-space(.)=${quote(text)}]`;
export const containingText = (tag, text) => `//${tag}[contains(normalize-space(.), ${quote(text)})]`;

function quote(text) {
  return text.includes('"') ? `'${text}'` : `"${text}"`;
}
