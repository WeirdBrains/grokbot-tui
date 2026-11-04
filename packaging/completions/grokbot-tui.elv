
use builtin;
use str;

set edit:completion:arg-completer[grokbot-tui] = {|@words|
    fn spaces {|n|
        builtin:repeat $n ' ' | str:join ''
    }
    fn cand {|text desc|
        edit:complex-candidate $text &display=$text' '(spaces (- 14 (wcswidth $text)))$desc
    }
    var command = 'grokbot-tui'
    for word $words[1..-1] {
        if (str:has-prefix $word '-') {
            break
        }
        set command = $command';'$word
    }
    var completions = [
        &'grokbot-tui'= {
            cand --account 'Account alias to start in (default: last active account)'
            cand --data-dir 'Data directory holding context.db (default ~/.grokbot-tui; a pre-1.2 ~/.gb is moved there)'
            cand --theme 'Use this theme (a file in <data dir>/themes, without .toml) for this run only; the ui.theme pref, F5 or /theme use set it for good'
            cand --mock 'Use the offline mock API (fake bots, echo replies; any token except "bad" works)'
            cand --list-themes 'List the themes in <data dir>/themes (● marks the one in use; broken files say why) and exit'
            cand -h 'Print help'
            cand --help 'Print help'
            cand -V 'Print version'
            cand --version 'Print version'
            cand mcp 'MCP server for other agents (Claude Code, Cursor, ...): `mcp serve [--stdio|--http [ADDR]]`, `mcp token`, `mcp config`'
            cand completions 'Print a shell completion script to stdout: bash, zsh, fish, powershell or elvish (e.g. `source <(grokbot-tui completions bash)`)'
            cand help 'Print this message or the help of the given subcommand(s)'
        }
        &'grokbot-tui;mcp'= {
            cand -h 'Print help'
            cand --help 'Print help'
            cand serve 'Run the MCP server (stdio by default; --http for streamable HTTP on localhost)'
            cand token 'Print the HTTP bearer token (created on first use) for a client''s config'
            cand config 'Show the MCP settings in use and where they live'
            cand help 'Print this message or the help of the given subcommand(s)'
        }
        &'grokbot-tui;mcp;serve'= {
            cand --http 'Serve streamable HTTP at ADDR (loopback only; default from mcp.toml, else 127.0.0.1:8765). Clients send `Authorization: Bearer <grokbot-tui mcp token>`'
            cand --stdio 'Serve over stdin/stdout (default)'
            cand --read-only 'Refuse send_message and answer_card for this run (overrides mcp.toml)'
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'grokbot-tui;mcp;token'= {
            cand --rotate 'Make a new token; clients using the old one stop working'
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'grokbot-tui;mcp;config'= {
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'grokbot-tui;mcp;help'= {
            cand serve 'Run the MCP server (stdio by default; --http for streamable HTTP on localhost)'
            cand token 'Print the HTTP bearer token (created on first use) for a client''s config'
            cand config 'Show the MCP settings in use and where they live'
            cand help 'Print this message or the help of the given subcommand(s)'
        }
        &'grokbot-tui;mcp;help;serve'= {
        }
        &'grokbot-tui;mcp;help;token'= {
        }
        &'grokbot-tui;mcp;help;config'= {
        }
        &'grokbot-tui;mcp;help;help'= {
        }
        &'grokbot-tui;completions'= {
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'grokbot-tui;help'= {
            cand mcp 'MCP server for other agents (Claude Code, Cursor, ...): `mcp serve [--stdio|--http [ADDR]]`, `mcp token`, `mcp config`'
            cand completions 'Print a shell completion script to stdout: bash, zsh, fish, powershell or elvish (e.g. `source <(grokbot-tui completions bash)`)'
            cand help 'Print this message or the help of the given subcommand(s)'
        }
        &'grokbot-tui;help;mcp'= {
            cand serve 'Run the MCP server (stdio by default; --http for streamable HTTP on localhost)'
            cand token 'Print the HTTP bearer token (created on first use) for a client''s config'
            cand config 'Show the MCP settings in use and where they live'
        }
        &'grokbot-tui;help;mcp;serve'= {
        }
        &'grokbot-tui;help;mcp;token'= {
        }
        &'grokbot-tui;help;mcp;config'= {
        }
        &'grokbot-tui;help;completions'= {
        }
        &'grokbot-tui;help;help'= {
        }
    ]
    $completions[$command]
}
