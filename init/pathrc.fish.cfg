function cd --description 'Run cd and then evaluate pathrc'
  command builtin cd "$@" && eval "$(pathrc collate)"
end
