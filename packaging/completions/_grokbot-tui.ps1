
using namespace System.Management.Automation
using namespace System.Management.Automation.Language

Register-ArgumentCompleter -Native -CommandName 'grokbot-tui' -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)

    $commandElements = $commandAst.CommandElements
    $command = @(
        'grokbot-tui'
        for ($i = 1; $i -lt $commandElements.Count; $i++) {
            $element = $commandElements[$i]
            if ($element -isnot [StringConstantExpressionAst] -or
                $element.StringConstantType -ne [StringConstantType]::BareWord -or
                $element.Value.StartsWith('-') -or
                $element.Value -eq $wordToComplete) {
                break
        }
        $element.Value
    }) -join ';'

    $completions = @(switch ($command) {
        'grokbot-tui' {
            [CompletionResult]::new('--account', '--account', [CompletionResultType]::ParameterName, 'Account alias to start in (default: last active account)')
            [CompletionResult]::new('--data-dir', '--data-dir', [CompletionResultType]::ParameterName, 'Data directory holding context.db (default ~/.grokbot-tui; a pre-1.2 ~/.gb is moved there)')
            [CompletionResult]::new('--theme', '--theme', [CompletionResultType]::ParameterName, 'Use this theme (a file in <data dir>/themes, without .toml) for this run only; the ui.theme pref, F5 or /theme use set it for good')
            [CompletionResult]::new('--mock', '--mock', [CompletionResultType]::ParameterName, 'Use the offline mock API (fake bots, echo replies; any token except "bad" works)')
            [CompletionResult]::new('--list-themes', '--list-themes', [CompletionResultType]::ParameterName, 'List the themes in <data dir>/themes (● marks the one in use; broken files say why) and exit')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('-V', '-V ', [CompletionResultType]::ParameterName, 'Print version')
            [CompletionResult]::new('--version', '--version', [CompletionResultType]::ParameterName, 'Print version')
            [CompletionResult]::new('mcp', 'mcp', [CompletionResultType]::ParameterValue, 'MCP server for other agents (Claude Code, Cursor, ...): `mcp serve [--stdio|--http [ADDR]]`, `mcp token`, `mcp config`')
            [CompletionResult]::new('completions', 'completions', [CompletionResultType]::ParameterValue, 'Print a shell completion script to stdout: bash, zsh, fish, powershell or elvish (e.g. `source <(grokbot-tui completions bash)`)')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'grokbot-tui;mcp' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('serve', 'serve', [CompletionResultType]::ParameterValue, 'Run the MCP server (stdio by default; --http for streamable HTTP on localhost)')
            [CompletionResult]::new('token', 'token', [CompletionResultType]::ParameterValue, 'Print the HTTP bearer token (created on first use) for a client''s config')
            [CompletionResult]::new('config', 'config', [CompletionResultType]::ParameterValue, 'Show the MCP settings in use and where they live')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'grokbot-tui;mcp;serve' {
            [CompletionResult]::new('--http', '--http', [CompletionResultType]::ParameterName, 'Serve streamable HTTP at ADDR (loopback only; default from mcp.toml, else 127.0.0.1:8765). Clients send `Authorization: Bearer <grokbot-tui mcp token>`')
            [CompletionResult]::new('--stdio', '--stdio', [CompletionResultType]::ParameterName, 'Serve over stdin/stdout (default)')
            [CompletionResult]::new('--read-only', '--read-only', [CompletionResultType]::ParameterName, 'Refuse send_message and answer_card for this run (overrides mcp.toml)')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'grokbot-tui;mcp;token' {
            [CompletionResult]::new('--rotate', '--rotate', [CompletionResultType]::ParameterName, 'Make a new token; clients using the old one stop working')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'grokbot-tui;mcp;config' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'grokbot-tui;mcp;help' {
            [CompletionResult]::new('serve', 'serve', [CompletionResultType]::ParameterValue, 'Run the MCP server (stdio by default; --http for streamable HTTP on localhost)')
            [CompletionResult]::new('token', 'token', [CompletionResultType]::ParameterValue, 'Print the HTTP bearer token (created on first use) for a client''s config')
            [CompletionResult]::new('config', 'config', [CompletionResultType]::ParameterValue, 'Show the MCP settings in use and where they live')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'grokbot-tui;mcp;help;serve' {
            break
        }
        'grokbot-tui;mcp;help;token' {
            break
        }
        'grokbot-tui;mcp;help;config' {
            break
        }
        'grokbot-tui;mcp;help;help' {
            break
        }
        'grokbot-tui;completions' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'grokbot-tui;help' {
            [CompletionResult]::new('mcp', 'mcp', [CompletionResultType]::ParameterValue, 'MCP server for other agents (Claude Code, Cursor, ...): `mcp serve [--stdio|--http [ADDR]]`, `mcp token`, `mcp config`')
            [CompletionResult]::new('completions', 'completions', [CompletionResultType]::ParameterValue, 'Print a shell completion script to stdout: bash, zsh, fish, powershell or elvish (e.g. `source <(grokbot-tui completions bash)`)')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'grokbot-tui;help;mcp' {
            [CompletionResult]::new('serve', 'serve', [CompletionResultType]::ParameterValue, 'Run the MCP server (stdio by default; --http for streamable HTTP on localhost)')
            [CompletionResult]::new('token', 'token', [CompletionResultType]::ParameterValue, 'Print the HTTP bearer token (created on first use) for a client''s config')
            [CompletionResult]::new('config', 'config', [CompletionResultType]::ParameterValue, 'Show the MCP settings in use and where they live')
            break
        }
        'grokbot-tui;help;mcp;serve' {
            break
        }
        'grokbot-tui;help;mcp;token' {
            break
        }
        'grokbot-tui;help;mcp;config' {
            break
        }
        'grokbot-tui;help;completions' {
            break
        }
        'grokbot-tui;help;help' {
            break
        }
    })

    $completions.Where{ $_.CompletionText -like "$wordToComplete*" } |
        Sort-Object -Property ListItemText
}
