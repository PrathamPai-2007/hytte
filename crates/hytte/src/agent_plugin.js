// Hytte: shows this OpenCode session on the pill (working / needs you / done), and puts its
// permission prompts there as Allow / Deny. Written by `hytte setup agents`; removed by
// `hytte setup agents --undo`. Every step is best-effort: a missing Hytte never slows OpenCode.
import { spawn } from "node:child_process";

const HYTTE = "__HYTTE_EXE__";

function hytte(args, input) {
  return new Promise((resolve) => {
    let out = "";
    try {
      const p = spawn(HYTTE, ["agent", ...args, "--name", "OpenCode"], {
        windowsHide: true,
        stdio: ["pipe", "pipe", "ignore"],
      });
      p.stdout.on("data", (d) => (out += d));
      p.on("error", () => resolve(""));
      p.on("close", () => resolve(out));
      p.stdin.on("error", () => {});
      p.stdin.end(input ?? "");
    } catch {
      resolve("");
    }
  });
}

export const Hytte = async () => {
  // Sub-agent sessions: their idle is not the end of the turn.
  const children = new Set();
  let last = "";
  const say = (cmd, input) => {
    if (cmd === last && !input) return;
    last = cmd;
    void hytte([cmd], input);
  };
  return {
    event: async ({ event }) => {
      const p = event?.properties ?? {};
      switch (event?.type) {
        case "session.created":
        case "session.updated":
          if (p.info?.parentID) children.add(p.info.id);
          break;
        case "session.status":
          if (p.status?.type === "busy" && !children.has(p.sessionID)) say("resume");
          break;
        case "session.idle":
          if (!children.has(p.sessionID)) say("done");
          break;
        case "permission.asked":
        case "permission.updated":
          say("needs-input", JSON.stringify({ message: p.title ?? `Wants to use ${p.permission ?? p.type ?? "a tool"}` }));
          break;
        case "permission.replied":
          say("resume");
          break;
      }
    },
    // Answered on the pill within 30 s, else OpenCode asks in the terminal as usual.
    "permission.ask": async (input, output) => {
      const pat = Array.isArray(input.pattern) ? input.pattern.join(" ") : input.pattern;
      const cmd = input.metadata?.command ?? pat;
      const payload =
        input.type === "bash" && cmd
          ? { tool_name: "Bash", tool_input: { command: cmd } }
          : { tool_name: input.title ?? input.type ?? "a tool" };
      const out = await hytte(["ask"], JSON.stringify(payload));
      try {
        const b = JSON.parse(out).hookSpecificOutput.decision.behavior;
        if (b === "allow" || b === "deny") output.status = b;
      } catch {}
    },
  };
};
