"""Plan edit_frame_html calls that turn a Doop frame into a freshly extracted one.

Usage: python syncframe.py <get_frame result file> <new html> <out dir>

The extractor numbers icon symbols in order of appearance, so one new icon renames every
later one. Renaming does not change the rendering, so the new document first reuses the
frame's existing symbol ids and order; only real changes are left to send.
Writes <out>/<name>.target.html (what the frame must equal) and <out>/<name>.edits.json.
"""
import difflib, json, os, re, sys

src, new_path, out_dir = sys.argv[1:4]
name = os.path.basename(new_path).removesuffix('.html')
raw = open(src, encoding='utf-8').read()
old = json.loads(raw[raw.index('{'):])['html']
new = open(new_path, encoding='utf-8').read()

SYM = re.compile(r'<symbol id="(i\d+)">(.*?)</symbol>', re.S)
old_syms = SYM.findall(old)
new_syms = SYM.findall(new)
old_by_body = {body: sid for sid, body in old_syms}
used = {sid for sid, _ in old_syms}
rename, fresh = {}, max([int(s[1:]) for s in used] + [-1]) + 1
for sid, body in new_syms:
    if body in old_by_body:
        rename[sid] = old_by_body[body]
    else:
        while f'i{fresh}' in used:
            fresh += 1
        rename[sid] = f'i{fresh}'
        used.add(rename[sid])
        fresh += 1

new_bodies = {body for _, body in new_syms}
kept = [(sid, body) for sid, body in old_syms if body in new_bodies]
added = [(rename[sid], body) for sid, body in new_syms if body not in old_by_body]
defs = ''.join(f'<symbol id="{sid}">{body}</symbol>' for sid, body in kept + added)

# Replace the defs block, then every reference, in one pass each.
target = re.sub(r'<defs>.*?</defs>', lambda m: f'<defs>{defs}</defs>', new, count=1, flags=re.S)
target = re.sub(r'href="#(i\d+)"', lambda m: f'href="#{rename.get(m.group(1), m.group(1))}"', target)

tok = lambda s: re.findall(r'[^{};<>]*[{};<>]|[^{};<>]+$', s)
a, b = tok(old), tok(target)
starts = [0]
for t in a:
    starts.append(starts[-1] + len(t))
ops = [op for op in difflib.SequenceMatcher(None, a, b, autojunk=False).get_opcodes() if op[0] != 'equal']

# Merge changes that sit close together: fewer calls for a little more text.
GAP = int(os.environ.get('GAP', '0'))
groups = []
for op in ops:
    if groups and starts[op[1]] - starts[groups[-1][2]] <= GAP:
        g = groups[-1]
        groups[-1] = ('replace', g[1], op[2], g[3], op[4])
    else:
        groups.append(op)
ops = groups

doc, edits = old, []
for tag, i1, i2, j1, j2 in reversed(ops):
    lo, hi = i1, i2
    while True:
        s, e = starts[lo], starts[hi]
        old_s = doc[s:e]
        if old_s and doc.count(old_s) == 1:
            break
        if lo > 0: lo -= 1
        if hi < len(a): hi += 1
    new_s = ''.join(a[lo:i1]) + ''.join(b[j1:j2]) + ''.join(a[i2:hi])
    doc = doc[:s] + new_s + doc[e:]
    edits.append({'old_str': old_s, 'new_str': new_s})

open(os.path.join(out_dir, f'{name}.target.html'), 'w', encoding='utf-8').write(target)
json.dump(edits, open(os.path.join(out_dir, f'{name}.edits.json'), 'w', encoding='utf-8'), ensure_ascii=False, indent=1)
print(f"{name}: {len(edits)} edits, {sum(len(x['old_str']) + len(x['new_str']) for x in edits)} chars, exact: {doc == target}")
