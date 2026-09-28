import { Fragment, type ReactNode } from "react";

/**
 * Just enough Markdown for chat answers: paragraphs, lists, headings, code,
 * simple tables, **bold** and `code`. Anything else shows as written, which
 * for short answers is fine — and it keeps a Markdown library out of the app.
 */
export default function ChatMarkdown({ text }: { text: string }) {
  return <div className="md">{blocks(text)}</div>;
}

function inline(text: string): ReactNode[] {
  const out: ReactNode[] = [];
  const re = /(\*\*[^*]+\*\*|`[^`]+`)/g;
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text))) {
    if (m.index > last) out.push(text.slice(last, m.index));
    const token = m[0];
    out.push(
      token.startsWith("**") ? (
        <strong key={m.index}>{token.slice(2, -2)}</strong>
      ) : (
        <code key={m.index}>{token.slice(1, -1)}</code>
      )
    );
    last = m.index + token.length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

const cells = (line: string) =>
  line
    .trim()
    .replace(/^\||\|$/g, "")
    .split("|")
    .map((c) => c.trim());

function blocks(text: string): ReactNode[] {
  const lines = text.replace(/\r\n/g, "\n").split("\n");
  const out: ReactNode[] = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];

    if (line.trimStart().startsWith("```")) {
      const body: string[] = [];
      i++;
      while (i < lines.length && !lines[i].trimStart().startsWith("```")) body.push(lines[i++]);
      i++;
      out.push(<pre key={out.length}>{body.join("\n")}</pre>);
      continue;
    }

    if (!line.trim()) {
      i++;
      continue;
    }

    const heading = /^(#{1,4})\s+(.*)$/.exec(line);
    if (heading) {
      out.push(
        <p key={out.length} className="md-h">
          {inline(heading[2])}
        </p>
      );
      i++;
      continue;
    }

    if (line.trimStart().startsWith("|")) {
      const rows: string[][] = [];
      while (i < lines.length && lines[i].trimStart().startsWith("|")) {
        const row = cells(lines[i++]);
        // The |---|---| line only says "the header ends here".
        if (!row.every((c) => /^:?-+:?$/.test(c))) rows.push(row);
      }
      const [head, ...body] = rows;
      out.push(
        <div key={out.length} className="md-table">
          <table>
            <thead>
              <tr>
                {head.map((c, j) => (
                  <th key={j}>{inline(c)}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {body.map((r, k) => (
                <tr key={k}>
                  {r.map((c, j) => (
                    <td key={j}>{inline(c)}</td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
      continue;
    }

    const bullet = /^\s*([-*・]|\d+[.)])\s+/;
    if (bullet.test(line)) {
      const ordered = /^\s*\d/.test(line);
      const items: string[] = [];
      while (i < lines.length && bullet.test(lines[i])) {
        items.push(lines[i++].replace(bullet, ""));
      }
      const List = ordered ? "ol" : "ul";
      out.push(
        <List key={out.length}>
          {items.map((it, j) => (
            <li key={j}>{inline(it)}</li>
          ))}
        </List>
      );
      continue;
    }

    const para: string[] = [];
    while (
      i < lines.length &&
      lines[i].trim() &&
      !bullet.test(lines[i]) &&
      !lines[i].trimStart().startsWith("```") &&
      !lines[i].trimStart().startsWith("|") &&
      !/^#{1,4}\s/.test(lines[i])
    ) {
      para.push(lines[i++]);
    }
    out.push(
      <p key={out.length}>
        {para.map((l, j) => (
          <Fragment key={j}>
            {j > 0 && <br />}
            {inline(l)}
          </Fragment>
        ))}
      </p>
    );
  }
  return out;
}
