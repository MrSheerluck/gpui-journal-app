












import { createServer } from "node:http";

const seen = new Set();

const server = createServer((req, res) => {
  const send = (status, body) => {
    res.writeHead(status, { "content-type": "application/json" });
    res.end(JSON.stringify(body));
  };

  if (req.method === "GET" && req.url === "/__seen") {
    return send(200, { emails: [...seen] });
  }

  if (!req.headers.authorization?.startsWith("Bearer ")) {
    return send(403, { success: false, errors: [{ message: "unauthorized" }] });
  }

  let body = "";
  req.on("data", (chunk) => (body += chunk));
  req.on("end", () => {
    const { sql = "", params = [] } = JSON.parse(body || "{}");
    const email = params[0];
    if (!sql.includes("INSERT") || typeof email !== "string") {
      return send(200, {
        success: false,
        errors: [{ message: `mock only handles waitlist inserts, got: ${sql}` }],
      });
    }

    const changes = seen.has(email) ? 0 : 1;
    seen.add(email);
    console.log(`mock d1: ${changes === 0 ? "duplicate" : "inserted"} ${email}`);
    send(200, { success: true, errors: [], result: [{ meta: { changes }, success: true }] });
  });
});

server.listen(8787, "127.0.0.1", () => console.log("mock d1 listening on 127.0.0.1:8787"));
