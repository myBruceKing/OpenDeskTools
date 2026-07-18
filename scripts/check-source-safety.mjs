import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import assert from "node:assert/strict";

const contentRules = [
  {
    name: "Windows user profile path",
    pattern: /[A-Za-z]:[\\/]Users[\\/][^\\/\s"']+/giu
  },
  {
    name: "Unix home directory path",
    pattern: /\/(?:Users|home)\/[^/\s"']+/gu
  },
  {
    name: "Windows developer workspace path",
    pattern: /[A-Za-z]:[\\/](?:Projects?|repos?|source|workspace)[\\/][^\\/\s"']+/giu
  },
  {
    name: "private key material",
    pattern: /-----BEGIN\s+(?:RSA\s+|EC\s+|OPENSSH\s+)?PRIVATE KEY-----/gu
  },
  {
    name: "OpenAI-style secret key",
    pattern: /\bsk-[A-Za-z0-9_-]{16,}\b/gu
  },
  {
    name: "GitHub token",
    pattern: /\b(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,})\b/gu
  },
  {
    name: "AWS access key",
    pattern: /\bAKIA[A-Z0-9]{16}\b/gu
  },
  {
    name: "non-placeholder account email",
    pattern: /\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.(?:ai|app|cloud|cn|co|com|dev|io|net|org|tech)\b/giu,
    allow: (value) =>
      /@example\.(?:com|net|org)$/iu.test(value) ||
      /^git@(?:bitbucket\.org|github\.com|gitlab\.com)$/iu.test(value)
  }
];

const fileNameRules = [
  /(^|[\\/])\.env(?!\.example(?:$|[\\/]))(?:\.|$)/iu,
  /(^|[\\/])id_(?:rsa|dsa|ecdsa|ed25519)(?:\.pub)?$/iu,
  /\.(?:p12|pfx)$/iu
];

function repositoryFiles() {
  const output = execFileSync(
    "git",
    ["ls-files", "--cached", "--others", "--exclude-standard", "-z"],
    { encoding: "buffer" }
  );
  return output
    .toString("utf8")
    .split("\0")
    .filter(Boolean);
}

function lineNumberAt(text, index) {
  let line = 1;
  for (let cursor = 0; cursor < index; cursor += 1) {
    if (text.charCodeAt(cursor) === 10) {
      line += 1;
    }
  }
  return line;
}

function scanFile(file) {
  const findings = [];
  if (fileNameRules.some((pattern) => pattern.test(file))) {
    findings.push({ file, line: 1, rule: "sensitive file name" });
  }

  const content = readFileSync(file);
  if (content.includes(0)) {
    return findings;
  }
  const text = content.toString("utf8");
  for (const rule of contentRules) {
    rule.pattern.lastIndex = 0;
    for (const match of text.matchAll(rule.pattern)) {
      if (rule.allow?.(match[0])) {
        continue;
      }
      findings.push({
        file,
        line: lineNumberAt(text, match.index ?? 0),
        rule: rule.name
      });
    }
  }
  return findings;
}

function runRuleSelfTest() {
  const scan = (text) => contentRules.flatMap((rule) => {
    rule.pattern.lastIndex = 0;
    return Array.from(text.matchAll(rule.pattern))
      .filter((match) => !rule.allow?.(match[0]))
      .map(() => rule.name);
  });

  assert.deepEqual(scan("icons/128x128@2x.png"), []);
  assert.deepEqual(scan("contact@example.com"), []);
  assert.deepEqual(scan("git@github.com:open-desk-tools/project.git"), []);
  assert.deepEqual(scan("git@gitlab.com:open-desk-tools/project.git"), []);
  assert.deepEqual(scan("git@bitbucket.org:open-desk-tools/project.git"), []);
  assert.deepEqual(
    scan("employee" + "@" + "company.com"),
    ["non-placeholder account email"]
  );
  assert.equal(fileNameRules.some((pattern) => pattern.test(".env.example")), false);
  assert.equal(fileNameRules.some((pattern) => pattern.test(".env.production")), true);
  assert.equal(fileNameRules.some((pattern) => pattern.test(".ENV")), true);
  assert.equal(fileNameRules.some((pattern) => pattern.test("ID_ED25519")), true);
  assert.deepEqual(
    scan(["C:", "Users", "developer", "project"].join("\\")),
    ["Windows user profile path"]
  );
  assert.deepEqual(
    scan(["D:", "Project", "OpenDeskTools"].join("\\")),
    ["Windows developer workspace path"]
  );
  assert.deepEqual(
    scan("token=" + "sk-" + "abcdefghijklmnop"),
    ["OpenAI-style secret key"]
  );
  assert.deepEqual(
    scan("token=" + "github_" + "pat_" + "abcdefghijklmnopqrstuv"),
    ["GitHub token"]
  );
}

runRuleSelfTest();
const findings = repositoryFiles().flatMap(scanFile);
if (findings.length > 0) {
  console.error("Source safety check failed:");
  for (const finding of findings) {
    console.error(`- ${finding.file}:${finding.line} (${finding.rule})`);
  }
  process.exitCode = 1;
} else {
  console.log("Source safety check passed.");
}
