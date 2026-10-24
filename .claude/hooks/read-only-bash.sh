#!/bin/bash
# The read-only guard of the review agents (.claude/agents/), a PreToolUse
# hook on Bash.  It refuses a command that would change the repo, a remote or
# a published package, by git, cargo, gh and a short list of other commands.
# It also refuses `xtask publish` and `xtask review`, which changes the state
# of a review round, also through `cargo run` of xtask.
# It matches text, so it stops mistakes, not an agent that tries to escape.
# Redirections and `sed -i` still pass.  Known gaps: TD-34 in docs/tech-debt.md.
# For bash 3.2 (macOS): no `declare -A`, no `${x,,}`, no `mapfile`.
# It uses no external program but jq.

# LC_ALL=C makes each character one byte, so the parse of a long command is
# fast.  No pattern depends on it: `lower` lists the capital letters, since
# bash 3.2 with a UTF-8 locale matches small letters with `[A-Z]`.
export LC_ALL=C

READ_ONLY_ADVICE="This agent is read only.  Do not try another form of the command."
PARSE_ADVICE="This is a limit of the guard's parse.  Write the command again without a backtick or an unmatched quote, for example with \`sed\`."
NL=$'\n'
TAB=$'\t'

refused=0
# Exits 2 with one line: "read-only guard: <label>: <text>.  <advice>".
refuse() {
  refused=1
  local advice=$READ_ONLY_ADVICE
  case $1 in
    backtick | "unclosed quote") advice=$PARSE_ADVICE ;;
  esac
  if [ $# -ge 2 ]; then
    printf 'read-only guard: %s: %s.  %s\n' "$1" "$(one_line "$2")" "$advice" >&2
  else
    printf 'read-only guard: %s.  %s\n' "$1" "$advice" >&2
  fi
  exit 2
}

# Any other exit that is not a pass is an error of the guard: exit 2.
on_exit() {
  local status=$?
  if [ "$status" -ne 0 ] && [ "$refused" -eq 0 ]; then
    printf 'read-only guard: the guard failed with status %s.  %s\n' "$status" "$READ_ONLY_ADVICE" >&2
    exit 2
  fi
}
trap on_exit EXIT

# The text with each new line as a space, and no blanks at its ends.
one_line() {
  local text=${1//$NL/ }
  text=${text#"${text%%[![:blank:]]*}"}
  text=${text%"${text##*[![:blank:]]}"}
  printf '%s' "$text"
}

cmd=$(jq -er '.tool_input.command | strings' 2>/dev/null) || refuse "cannot read the command"

# The parse, in one pass.  Each word goes into WORDS; part p holds the words
# from PART_START[p] to PART_END[p] (exclusive), and its text is PART_TEXT[p].
WORDS=()
PART_START=()
PART_END=()
PART_TEXT=()
word=""
started=0
first_word=0
part_from=0

end_word() {
  if [ $started -eq 1 ]; then
    WORDS[${#WORDS[@]}]=$word
  fi
  word=""
  started=0
}

# Ends the part before position $1 of the command.  An empty part is skipped.
end_part() {
  end_word
  if [ ${#WORDS[@]} -gt $first_word ]; then
    PART_START[${#PART_START[@]}]=$first_word
    PART_END[${#PART_END[@]}]=${#WORDS[@]}
    PART_TEXT[${#PART_TEXT[@]}]=${cmd:part_from:$1-part_from}
  fi
  first_word=${#WORDS[@]}
  part_from=$(($1 + 1))
}

state=none
n=${#cmd}
i=0
while [ $i -lt $n ]; do
  c=${cmd:i:1}
  if [ $state = single ]; then
    if [ "$c" = "'" ]; then state=none; else word=$word$c; fi
    i=$((i + 1))
    continue
  fi
  if [ "$c" = '`' ]; then refuse backtick "$cmd"; fi
  if [ "$c" = '\' ]; then
    next=${cmd:i+1:1}
    if [ "$next" = '`' ]; then refuse backtick "$cmd"; fi
    if [ "$next" = "$NL" ]; then
      i=$((i + 2))
      continue
    fi
    if [ $((i + 1)) -lt $n ]; then word=$word$next; else word=$word$c; fi
    started=1
    i=$((i + 2))
    continue
  fi
  if [ $state = double ]; then
    if [ "$c" = '"' ]; then state=none; else word=$word$c; fi
    i=$((i + 1))
    continue
  fi
  case $c in
    "'")
      state=single
      started=1
      ;;
    '"')
      state=double
      started=1
      ;;
    '#')
      if [ $started -eq 0 ]; then
        while [ $i -lt $n ] && [ "${cmd:i:1}" != "$NL" ]; do i=$((i + 1)); done
        continue
      fi
      word=$word$c
      ;;
    ';' | '&' | '|' | '(' | ')' | "$NL") end_part $i ;;
    ' ' | "$TAB") end_word ;;
    *)
      word=$word$c
      started=1
      ;;
  esac
  i=$((i + 1))
done
if [ $state != none ]; then refuse "unclosed quote" "$cmd"; fi
end_part $n

CAPITALS=ABCDEFGHIJKLMNOPQRSTUVWXYZ
SMALL_LETTERS=abcdefghijklmnopqrstuvwxyz

# Sets LOWERED to $1 with each capital letter as a small letter.  The file
# system of macOS ignores letter case, so `GIT` runs git.
lower() {
  LOWERED=$1
  case $1 in
    *[ABCDEFGHIJKLMNOPQRSTUVWXYZ]*) ;;
    *) return 0 ;;
  esac
  LOWERED=""
  local j=0 c before
  while [ $j -lt ${#1} ]; do
    c=${1:j:1}
    case $c in
      [ABCDEFGHIJKLMNOPQRSTUVWXYZ])
        before=${CAPITALS%%"$c"*}
        c=${SMALL_LETTERS:${#before}:1}
        ;;
    esac
    LOWERED=$LOWERED$c
    j=$((j + 1))
  done
}

# True if word $1 is NAME=value or NAME+=value.  Sets NAME.
is_assignment() {
  case $1 in
    *=*) ;;
    *) return 1 ;;
  esac
  NAME=${1%%=*}
  NAME=${NAME%+}
  case $NAME in
    '' | [!A-Za-z_]* | *[!A-Za-z0-9_]*) return 1 ;;
  esac
  return 0
}

# True if a word from $2 to $3 (exclusive) is exactly $1.
has_word() {
  local j=$2
  while [ $j -lt $3 ]; do
    if [ "${WORDS[j]}" = "$1" ]; then return 0; fi
    j=$((j + 1))
  done
  return 1
}

# Refuses with label $1 if a word from $3 to $4 (exclusive) starts with $2.
refuse_prefix() {
  local j=$3
  while [ $j -lt $4 ]; do
    case ${WORDS[j]} in
      "$2"*) refuse "$1" "$text" ;;
    esac
    j=$((j + 1))
  done
}

# True if a word from $1 to $2 (exclusive) names the xtask package or binary,
# as in `-p xtask`, `-pxtask`, `--package=xtask` or `xtask/Cargo.toml`.
names_xtask() {
  local j=$1
  while [ $j -lt $2 ]; do
    case ${WORDS[j]} in
      xtask | -pxtask | *=xtask | xtask/Cargo.toml | */xtask/Cargo.toml | *=xtask/Cargo.toml) return 0 ;;
    esac
    j=$((j + 1))
  done
  return 1
}

# Refuses an xtask command that pushes (`publish`) or that changes the state
# of a review round (`review`), if a word from $1 to $2 (exclusive) is one.
check_xtask_words() {
  if has_word publish $1 $2 || has_word review $1 $2; then refuse command "$text"; fi
}

check_cargo() {
  refuse_prefix "cargo option" --config $((k + 1)) $end
  local j=$((k + 1))
  while [ $j -lt $end ]; do
    case ${WORDS[j]} in
      --color | -Z | -C) j=$((j + 2)) ;;
      -* | +*) j=$((j + 1)) ;;
      *) break ;;
    esac
  done
  local sub=""
  if [ $j -lt $end ]; then sub=${WORDS[j]}; fi
  case $sub in
    publish | login | owner | yank | install | uninstall | add | remove | rm | update | clean | search | fix | logout | generate-lockfile | vendor)
      refuse "cargo subcommand" "$text"
      ;;
    fmt)
      if ! has_word --check $((k + 1)) $end; then refuse "cargo subcommand" "$text"; fi
      ;;
    run | r)
      if names_xtask $((j + 1)) $end; then check_xtask_words $((j + 1)) $end; fi
      ;;
  esac
  if has_word --fix $((k + 1)) $end; then refuse "cargo subcommand" "$text"; fi
}

check_git() {
  local j=$((k + 1))
  while [ $j -lt $end ]; do
    case ${WORDS[j]} in
      -c* | --config-env*) refuse "git option" "$text" ;;
      -C | --git-dir | --work-tree | --namespace) j=$((j + 2)) ;;
      -*) j=$((j + 1)) ;;
      *) break ;;
    esac
  done
  refuse_prefix "git output" --output $((k + 1)) $end
  if [ $j -ge $end ]; then return 0; fi
  case ${WORDS[j]} in
    diff | log | show | status | blame | grep | ls-files | ls-tree | rev-parse | rev-list | merge-base | cat-file | show-ref | for-each-ref | describe | worktree) ;;
    *) refuse "git subcommand" "$text" ;;
  esac
}

# Moves k past the words from k that start with `-`.
skip_options() {
  while [ $k -lt $end ]; do
    case ${WORDS[k]} in
      -*) k=$((k + 1)) ;;
      *) break ;;
    esac
  done
}

# Checks the part $1: strips its prefixes, then checks its first word.
check_part() {
  k=${PART_START[$1]}
  end=${PART_END[$1]}
  text=${PART_TEXT[$1]}
  while [ $k -lt $end ]; do
    lower "${WORDS[k]##*/}"
    case $LOWERED in
      rtk)
        k=$((k + 1))
        skip_options
        if [ $k -lt $end ] && [ "${WORDS[k]}" = proxy ]; then k=$((k + 1)); fi
        continue
        ;;
      timeout)
        k=$((k + 1))
        while [ $k -lt $end ]; do
          case ${WORDS[k]} in
            -s | -k | --signal | --kill-after) k=$((k + 2)) ;;
            -*) k=$((k + 1)) ;;
            *) break ;;
          esac
        done
        k=$((k + 1))
        continue
        ;;
      env | command | exec | time | nohup | export | declare | typeset | readonly | local)
        k=$((k + 1))
        while [ $k -lt $end ]; do
          case ${WORDS[k]} in
            -*) k=$((k + 1)) ;;
            *) break ;;
          esac
        done
        continue
        ;;
      '!' | '{' | if | then | elif | else | do | while | until)
        k=$((k + 1))
        continue
        ;;
    esac
    if is_assignment "${WORDS[k]}"; then
      case $NAME in
        GIT_* | UPDATE_SNAPSHOTS) refuse variable "$text" ;;
      esac
      k=$((k + 1))
      continue
    fi
    break
  done
  if [ $k -ge $end ]; then return 0; fi
  lower "${WORDS[k]##*/}"
  case $LOWERED in
    gh | curl | wget | bash | sh | zsh | eval | xargs) refuse command "$text" ;;
    xtask) check_xtask_words $((k + 1)) $end ;;
    cargo) check_cargo ;;
    git) check_git ;;
  esac
}

p=0
while [ $p -lt ${#PART_START[@]} ]; do
  check_part $p
  p=$((p + 1))
done
exit 0
