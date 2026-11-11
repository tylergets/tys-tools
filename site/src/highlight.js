// Tiny syntax highlighter for the site's code blocks: YAML configs, shell usage and Nix
// snippets. Each line is scanned into tokens and rendered as <span class="tok-…">. Text is
// escaped here, so the result is safe to insert as HTML.

const escape = (text) =>
  text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

const span = (kind, text) => (kind ? `<span class="tok-${kind}">${escape(text)}</span>` : escape(text));

/// Scan `line` with `rules` (ordered [kind, sticky regex]) into tokens; unmatched text is plain.
function scan(line, rules) {
  const tokens = [];
  let at = 0;
  let plain = "";
  while (at < line.length) {
    let hit = null;
    for (const [kind, re] of rules) {
      re.lastIndex = at;
      const m = re.exec(line);
      if (m && m[0].length > 0) {
        hit = [kind, m[0]];
        break;
      }
    }
    if (hit) {
      if (plain) tokens.push([null, plain]), (plain = "");
      tokens.push(hit);
      at += hit[1].length;
    } else {
      plain += line[at++];
    }
  }
  if (plain) tokens.push([null, plain]);
  return tokens;
}

const render = (tokens) => tokens.map(([kind, text]) => span(kind, text)).join("");

// YAML: comments, `key:` at the start of a line or in a flow mapping, strings, numbers,
// booleans/null, and flow punctuation. Everything else is a plain scalar.
const yamlRules = [
  ["comment", /#.*$/y],
  ["key", /(?<=^\s*(?:- )?|[{,]\s*)[\w.\-"']+(?=\s*:(?:\s|$))/y],
  ["punct", /:(?=\s|$)|^\s*-(?=\s)|[{}\[\],]/y],
  ["str", /"(?:[^"\\]|\\.)*"|'(?:[^']|'')*'/y],
  ["bool", /(?<![\w.-])(?:true|false|null|yes|no|~)(?![\w.-])/y],
  ["num", /(?<![\w.-])[-+]?\d+(?:\.\d+)?(?![\w.-])/y],
];

const yaml = (line) => render(scan(line, yamlRules));

// Shell: the first word of a command is the program; flags, [optional] and <placeholder>
// arguments, redirects and comments stand out. Lines run by a known program get a prompt.
const PROMPT = /^(?:ty[a-z-]*|nix|cargo|npm|git)$/;
const shellRules = [
  ["comment", /(?<=^|\s)#.*$/y],
  ["cmd", /^\s*[\w./-]+/y],
  ["flag", /(?<=\s)--?[a-z][\w-]*/y],
  ["arg", /\[[^\]]*\]|<[^>]*>/y],
  ["str", /"(?:[^"\\]|\\.)*"|'[^']*'/y],
  ["punct", /(?<=\s)(?:\||&&|>|<|;)(?=\s|$)/y],
];

function shell(line) {
  const tokens = scan(line, shellRules);
  const cmd = tokens.find(([kind]) => kind === "cmd");
  const prompt = cmd && PROMPT.test(cmd[1].trim()) ? `<span class="tok-prompt">$ </span>` : "";
  return prompt + render(tokens);
}

// Nix: enough for flake snippets.
const nixRules = [
  ["comment", /#.*$/y],
  ["str", /"(?:[^"\\]|\\.)*"/y],
  ["bool", /(?<![\w.-])(?:true|false|null)(?![\w.-])/y],
  ["punct", /[=;{}\[\]]/y],
];

const nix = (line) => render(scan(line, nixRules));

const languages = { yaml, shell, nix };

/// Highlight `code` as `lang` ("yaml" | "shell" | "nix"); unknown languages are escaped only.
export function highlight(code, lang) {
  const line = languages[lang] ?? escape;
  return code.split("\n").map(line).join("\n");
}
