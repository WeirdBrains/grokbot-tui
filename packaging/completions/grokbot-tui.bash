_grokbot__tui() {
    local i cur prev opts cmd
    COMPREPLY=()
    if [[ "${BASH_VERSINFO[0]}" -ge 4 ]]; then
        cur="$2"
    else
        cur="${COMP_WORDS[COMP_CWORD]}"
    fi
    prev="$3"
    cmd=""
    opts=""

    for i in "${COMP_WORDS[@]:0:COMP_CWORD}"
    do
        case "${cmd},${i}" in
            ",$1")
                cmd="grokbot__tui"
                ;;
            grokbot__tui,completions)
                cmd="grokbot__tui__subcmd__completions"
                ;;
            grokbot__tui,help)
                cmd="grokbot__tui__subcmd__help"
                ;;
            grokbot__tui,mcp)
                cmd="grokbot__tui__subcmd__mcp"
                ;;
            grokbot__tui__subcmd__help,completions)
                cmd="grokbot__tui__subcmd__help__subcmd__completions"
                ;;
            grokbot__tui__subcmd__help,help)
                cmd="grokbot__tui__subcmd__help__subcmd__help"
                ;;
            grokbot__tui__subcmd__help,mcp)
                cmd="grokbot__tui__subcmd__help__subcmd__mcp"
                ;;
            grokbot__tui__subcmd__help__subcmd__mcp,config)
                cmd="grokbot__tui__subcmd__help__subcmd__mcp__subcmd__config"
                ;;
            grokbot__tui__subcmd__help__subcmd__mcp,serve)
                cmd="grokbot__tui__subcmd__help__subcmd__mcp__subcmd__serve"
                ;;
            grokbot__tui__subcmd__help__subcmd__mcp,token)
                cmd="grokbot__tui__subcmd__help__subcmd__mcp__subcmd__token"
                ;;
            grokbot__tui__subcmd__mcp,config)
                cmd="grokbot__tui__subcmd__mcp__subcmd__config"
                ;;
            grokbot__tui__subcmd__mcp,help)
                cmd="grokbot__tui__subcmd__mcp__subcmd__help"
                ;;
            grokbot__tui__subcmd__mcp,serve)
                cmd="grokbot__tui__subcmd__mcp__subcmd__serve"
                ;;
            grokbot__tui__subcmd__mcp,token)
                cmd="grokbot__tui__subcmd__mcp__subcmd__token"
                ;;
            grokbot__tui__subcmd__mcp__subcmd__help,config)
                cmd="grokbot__tui__subcmd__mcp__subcmd__help__subcmd__config"
                ;;
            grokbot__tui__subcmd__mcp__subcmd__help,help)
                cmd="grokbot__tui__subcmd__mcp__subcmd__help__subcmd__help"
                ;;
            grokbot__tui__subcmd__mcp__subcmd__help,serve)
                cmd="grokbot__tui__subcmd__mcp__subcmd__help__subcmd__serve"
                ;;
            grokbot__tui__subcmd__mcp__subcmd__help,token)
                cmd="grokbot__tui__subcmd__mcp__subcmd__help__subcmd__token"
                ;;
            *)
                ;;
        esac
    done

    case "${cmd}" in
        grokbot__tui)
            opts="-h -V --mock --account --data-dir --theme --list-themes --help --version mcp completions help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 1 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --account)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --data-dir)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --theme)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        grokbot__tui__subcmd__completions)
            opts="-h --help bash elvish fish powershell zsh"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        grokbot__tui__subcmd__help)
            opts="mcp completions help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        grokbot__tui__subcmd__help__subcmd__completions)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        grokbot__tui__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        grokbot__tui__subcmd__help__subcmd__mcp)
            opts="serve token config"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        grokbot__tui__subcmd__help__subcmd__mcp__subcmd__config)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        grokbot__tui__subcmd__help__subcmd__mcp__subcmd__serve)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        grokbot__tui__subcmd__help__subcmd__mcp__subcmd__token)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        grokbot__tui__subcmd__mcp)
            opts="-h --help serve token config help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        grokbot__tui__subcmd__mcp__subcmd__config)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        grokbot__tui__subcmd__mcp__subcmd__help)
            opts="serve token config help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        grokbot__tui__subcmd__mcp__subcmd__help__subcmd__config)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        grokbot__tui__subcmd__mcp__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        grokbot__tui__subcmd__mcp__subcmd__help__subcmd__serve)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        grokbot__tui__subcmd__mcp__subcmd__help__subcmd__token)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        grokbot__tui__subcmd__mcp__subcmd__serve)
            opts="-h --stdio --http --read-only --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --http)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        grokbot__tui__subcmd__mcp__subcmd__token)
            opts="-h --rotate --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
    esac
}

if [[ "${BASH_VERSINFO[0]}" -eq 4 && "${BASH_VERSINFO[1]}" -ge 4 || "${BASH_VERSINFO[0]}" -gt 4 ]]; then
    complete -F _grokbot__tui -o nosort -o bashdefault -o default grokbot-tui
else
    complete -F _grokbot__tui -o bashdefault -o default grokbot-tui
fi
