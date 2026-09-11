{ pkgs, ... }:

{
  # `process-compose` is devcroft's own supervisor, and it must come from
  # this project's closure — devcroft never takes it from the host's
  # PATH. Any project declaring services supplies it, exactly as the flox
  # sample installs it into its own manifest.
  packages = [ pkgs.python3 pkgs.process-compose ];

  # A real service, with its port supplied through `env` rather than
  # hardcoded in the command — the same shape `flox-services-sample`
  # uses, so the two can be compared field for field.
  processes.api = {
    exec = "python3 -m http.server $API_PORT --bind 127.0.0.1";
    env.API_PORT = "8730";
  };

  # A second process, ordered after the first.
  #
  # Note the spelling: `devenv:processes:api`, not `api`. Both are
  # accepted by devenv's own evaluation and only this one does anything —
  # `before`/`after` are edges in devenv's *task* graph, and a bare name
  # matches no node. devcroft refuses the bare form rather than guessing
  # what you meant. See README.md, "The spelling that looks right".
  processes.probe = {
    exec = "while true; do date -u; sleep 10; done";
    after = [ "devenv:processes:api" ];
  };
}
