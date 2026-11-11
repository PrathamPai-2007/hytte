# 9. Ask (local AI)

Ask a small AI model a question without leaving the pill. The model runs on your own PC through [Ollama](https://ollama.com), so nothing you ask, copy or open is sent to anyone.

Ask is **off by default**. Hytte never installs Ollama or a model by itself.

## Turn it on

1. Install Ollama from [ollama.com](https://ollama.com) and start it.
2. In [Settings](07-settings.md), switch on **Ask a model on this PC (Ollama)**. An **Ask** tab appears.
3. Open the tab. If the model isn't there yet, press **Get it**: Hytte asks your Ollama to download it (1.4 GB for the default, `qwen3:1.7b`) and shows the progress. **Stop** cancels; Ollama keeps what it has.

The default is small on purpose: it runs on an ordinary laptop. For a different one, pick **qwen3 0.6B** (smaller, weaker) or **gemma3 1B** in Settings, or choose **Custom** and set `[ai] model` to anything you have pulled with Ollama.

## Ask a question

Type in the field and press **Enter** (or **Ask**). The answer streams into the box below it; scroll it with the mouse wheel. **Stop** ends an answer early. **Copy** takes the whole answer and **Clear** forgets it. It is one question and one answer, not a chat: nothing is remembered between questions.

The first question after a while is slow, because Ollama has to load the model into memory (several seconds on a laptop without a graphics card, longer for a bigger model). Three dots show while it loads. Ollama keeps the model loaded for 2 minutes afterwards (`[ai] keep_alive`, or **Keep it in memory** in Settings), so the next question is quick.

To skip the wait, press **Load model now** on the empty pane: Ollama loads the model without being asked anything. It then frees it by itself after the same idle time (nothing in Hytte has to remember to unload it; `keep_alive = "-1"` keeps it loaded until Ollama quits).

With `[ai] hotkey` set (for example `Win+Alt+A`), the chord opens the Ask pane with the cursor ready.

## Explain, Summarise, Shorten

Anything Hytte already holds can be sent to the model in one click. The answer comes in a **card** that grows to fit it (it scrolls inside the card once it is about half the screen tall). **Copy** takes it, **Dismiss** closes it, and it folds away by itself a minute after it finishes.

- **A clipboard clip** (when [clipboard history](05-shelf.md#clipboard-history) is on): hover the clip and click the **✦**, then choose **Explain**, **Summarise** or **Shorten**.
- **A text file on the shelf** (txt, md, log, csv, json, yaml, toml, common code files): select its tile, then **Summarise** or **Explain**. Only the first 6 KB is sent.
- **A failed command** that ran through `hytte run` (it keeps its output): **Explain** next to **Copy error** says what most likely went wrong and what to try.

An agent's question (Allow / Deny) always comes first; an answer you asked for waits behind it.

## Privacy

- Only Ollama **on this PC** is contacted. `[ai] host` must be `127.0.0.1`, `localhost` or `[::1]`; any other address is refused, so text can't be sent to another machine by a typo.
- Hytte talks to it through the `curl.exe` that ships with Windows, with proxies switched off. Your question is passed on curl's standard input, so it never shows in a process list.
- Nothing is written to disk. Questions and answers live in memory and go when you press **Clear**, switch Ask off or quit.
- Clip text goes to the model only when you click an action on it. Hytte masks nothing in it, so don't ask about a password you copied; Ollama runs locally, but your own clipboard history rules still apply.

## What small models are good at

A model this small is good at: explaining an error message, summarising a page of text, rewriting a sentence shorter. It is poor at facts, arithmetic, long code and anything that must be right. Each answer says **local, can be wrong**, and it can be.

## If it doesn't work

| The pane says | What to do |
|---|---|
| **Ollama isn't running** | Start Ollama (it sits in the tray), then **Try again**. |
| **isn't downloaded** | **Get it**, or run `ollama pull <model>` in a terminal. |
| **Can't use Ollama** | Check `[ai] host`. It must be this PC. The default is `127.0.0.1:11434`. |
| An answer ends with *model not found* | The model name in `[ai] model` isn't one Ollama has. |

Next: back to [the manual](README.md).
