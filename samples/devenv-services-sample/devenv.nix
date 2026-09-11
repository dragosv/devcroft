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

    # When this counts as *ready*, which is not the same as started.
    # devcroft carries this to its supervisor, and — the part that makes
    # it worth declaring — `probe` below waits for ready rather than for
    # spawned. Without it, a dependent starts against a server that has
    # not bound its port yet.
    ready = {
      http.get = { host = "127.0.0.1"; port = 8730; path = "/"; };
      period = 1;
    };
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
