# settings.zsh: values from the private settings file (dots settings) that the
# shell needs. ~/.zshrc sources it after local.zsh, so these values win.
# Now: JAVA_HOME. The file format is in settings.example; the last line wins.
() {
  local f="$HOME/.config/dotfiles/settings" line v=""
  [[ -r $f ]] || return 0
  while IFS= read -r line || [[ -n $line ]]; do
    [[ $line == JAVA_HOME=* ]] || continue
    v=${line#JAVA_HOME=}
    v=${v%"${v##*[![:space:]]}"}
    v=${v#\"}; v=${v%\"}
  done <"$f"
  if [[ -n $v ]]; then export JAVA_HOME=$v; fi
}
