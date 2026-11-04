# Print an optspec for argparse to handle cmd's options that are independent of any subcommand.
function __fish_grokbot_tui_global_optspecs
    string join \n mock account= data-dir= theme= list-themes h/help V/version
end

function __fish_grokbot_tui_needs_command
    # Figure out if the current invocation already has a command.
    set -l cmd (commandline -opc)
    set -e cmd[1]
    argparse -s (__fish_grokbot_tui_global_optspecs) -- $cmd 2>/dev/null
    or return
    if set -q argv[1]
        # Also print the command, so this can be used to figure out what it is.
        echo $argv[1]
        return 1
    end
    return 0
end

function __fish_grokbot_tui_using_subcommand
    set -l cmd (__fish_grokbot_tui_needs_command)
    test -z "$cmd"
    and return 1
    contains -- $cmd[1] $argv
end

complete -c grokbot-tui -n "__fish_grokbot_tui_needs_command" -l account -d 'Account alias to start in (default: last active account)' -r
complete -c grokbot-tui -n "__fish_grokbot_tui_needs_command" -l data-dir -d 'Data directory holding context.db (default ~/.grokbot-tui; a pre-1.2 ~/.gb is moved there)' -r -F
complete -c grokbot-tui -n "__fish_grokbot_tui_needs_command" -l theme -d 'Use this theme (a file in <data dir>/themes, without .toml) for this run only; the ui.theme pref, F5 or /theme use set it for good' -r
complete -c grokbot-tui -n "__fish_grokbot_tui_needs_command" -l mock -d 'Use the offline mock API (fake bots, echo replies; any token except "bad" works)'
complete -c grokbot-tui -n "__fish_grokbot_tui_needs_command" -l list-themes -d 'List the themes in <data dir>/themes (● marks the one in use; broken files say why) and exit'
complete -c grokbot-tui -n "__fish_grokbot_tui_needs_command" -s h -l help -d 'Print help'
complete -c grokbot-tui -n "__fish_grokbot_tui_needs_command" -s V -l version -d 'Print version'
complete -c grokbot-tui -n "__fish_grokbot_tui_needs_command" -f -a "mcp" -d 'MCP server for other agents (Claude Code, Cursor, ...): `mcp serve [--stdio|--http [ADDR]]`, `mcp token`, `mcp config`'
complete -c grokbot-tui -n "__fish_grokbot_tui_needs_command" -f -a "completions" -d 'Print a shell completion script to stdout: bash, zsh, fish, powershell or elvish (e.g. `source <(grokbot-tui completions bash)`)'
complete -c grokbot-tui -n "__fish_grokbot_tui_needs_command" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c grokbot-tui -n "__fish_grokbot_tui_using_subcommand mcp; and not __fish_seen_subcommand_from serve token config help" -s h -l help -d 'Print help'
complete -c grokbot-tui -n "__fish_grokbot_tui_using_subcommand mcp; and not __fish_seen_subcommand_from serve token config help" -f -a "serve" -d 'Run the MCP server (stdio by default; --http for streamable HTTP on localhost)'
complete -c grokbot-tui -n "__fish_grokbot_tui_using_subcommand mcp; and not __fish_seen_subcommand_from serve token config help" -f -a "token" -d 'Print the HTTP bearer token (created on first use) for a client\'s config'
complete -c grokbot-tui -n "__fish_grokbot_tui_using_subcommand mcp; and not __fish_seen_subcommand_from serve token config help" -f -a "config" -d 'Show the MCP settings in use and where they live'
complete -c grokbot-tui -n "__fish_grokbot_tui_using_subcommand mcp; and not __fish_seen_subcommand_from serve token config help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c grokbot-tui -n "__fish_grokbot_tui_using_subcommand mcp; and __fish_seen_subcommand_from serve" -l http -d 'Serve streamable HTTP at ADDR (loopback only; default from mcp.toml, else 127.0.0.1:8765). Clients send `Authorization: Bearer <grokbot-tui mcp token>`' -r
complete -c grokbot-tui -n "__fish_grokbot_tui_using_subcommand mcp; and __fish_seen_subcommand_from serve" -l stdio -d 'Serve over stdin/stdout (default)'
complete -c grokbot-tui -n "__fish_grokbot_tui_using_subcommand mcp; and __fish_seen_subcommand_from serve" -l read-only -d 'Refuse send_message and answer_card for this run (overrides mcp.toml)'
complete -c grokbot-tui -n "__fish_grokbot_tui_using_subcommand mcp; and __fish_seen_subcommand_from serve" -s h -l help -d 'Print help'
complete -c grokbot-tui -n "__fish_grokbot_tui_using_subcommand mcp; and __fish_seen_subcommand_from token" -l rotate -d 'Make a new token; clients using the old one stop working'
complete -c grokbot-tui -n "__fish_grokbot_tui_using_subcommand mcp; and __fish_seen_subcommand_from token" -s h -l help -d 'Print help'
complete -c grokbot-tui -n "__fish_grokbot_tui_using_subcommand mcp; and __fish_seen_subcommand_from config" -s h -l help -d 'Print help'
complete -c grokbot-tui -n "__fish_grokbot_tui_using_subcommand mcp; and __fish_seen_subcommand_from help" -f -a "serve" -d 'Run the MCP server (stdio by default; --http for streamable HTTP on localhost)'
complete -c grokbot-tui -n "__fish_grokbot_tui_using_subcommand mcp; and __fish_seen_subcommand_from help" -f -a "token" -d 'Print the HTTP bearer token (created on first use) for a client\'s config'
complete -c grokbot-tui -n "__fish_grokbot_tui_using_subcommand mcp; and __fish_seen_subcommand_from help" -f -a "config" -d 'Show the MCP settings in use and where they live'
complete -c grokbot-tui -n "__fish_grokbot_tui_using_subcommand mcp; and __fish_seen_subcommand_from help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c grokbot-tui -n "__fish_grokbot_tui_using_subcommand completions" -s h -l help -d 'Print help'
complete -c grokbot-tui -n "__fish_grokbot_tui_using_subcommand help; and not __fish_seen_subcommand_from mcp completions help" -f -a "mcp" -d 'MCP server for other agents (Claude Code, Cursor, ...): `mcp serve [--stdio|--http [ADDR]]`, `mcp token`, `mcp config`'
complete -c grokbot-tui -n "__fish_grokbot_tui_using_subcommand help; and not __fish_seen_subcommand_from mcp completions help" -f -a "completions" -d 'Print a shell completion script to stdout: bash, zsh, fish, powershell or elvish (e.g. `source <(grokbot-tui completions bash)`)'
complete -c grokbot-tui -n "__fish_grokbot_tui_using_subcommand help; and not __fish_seen_subcommand_from mcp completions help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c grokbot-tui -n "__fish_grokbot_tui_using_subcommand help; and __fish_seen_subcommand_from mcp" -f -a "serve" -d 'Run the MCP server (stdio by default; --http for streamable HTTP on localhost)'
complete -c grokbot-tui -n "__fish_grokbot_tui_using_subcommand help; and __fish_seen_subcommand_from mcp" -f -a "token" -d 'Print the HTTP bearer token (created on first use) for a client\'s config'
complete -c grokbot-tui -n "__fish_grokbot_tui_using_subcommand help; and __fish_seen_subcommand_from mcp" -f -a "config" -d 'Show the MCP settings in use and where they live'
