{ pkgs, ... }:

{
  # Two packages, so a session can prove it is running the closure's
  # tooling rather than the host's — plus the service supervisor, which
  # any project declaring services must supply from its own closure
  # (devcroft never takes it from the host's PATH).
  packages = [ pkgs.ripgrep pkgs.jq pkgs.process-compose ];

  # Declared environment. Captured host-side at `devcroft up`, before any
  # restriction applies — the same phase flox, nix and devbox capture in.
  env.DEVCROFT_SAMPLE = "devenv";

  # Project code. devcroft never runs this on the host: it is captured as
  # data at `up` and executed *inside* the sandbox, after restriction.
  # The marker below is what makes that visible — see README.md.
  enterShell = ''
    mkdir -p .devcroft-sample
    date -u +%Y-%m-%dT%H:%M:%SZ > .devcroft-sample/enter-shell-ran
  '';

  # Supervised services. Read at `up` through `devenv eval processes`,
  # which runs no project code — the property that qualified devenv where
  # devbox does not (`devbox services ls` runs its init hook). The
  # commands themselves are project code and run *inside* the sandbox,
  # supervised by this sandbox's keeper and reaped at `devcroft down`.
  processes.ticker.exec = "while true; do date -u; sleep 5; done";

  # Ordering, in the one spelling that means the same thing to devenv and
  # to devcroft: a `devenv:processes:` task reference. A bare `"ticker"`
  # is refused rather than honoured — devenv itself creates no ordering
  # from it, and devcroft will not be more featureful than the provider
  # it is reading. See README.md.
  processes.follower = {
    exec = "while true; do echo following; sleep 5; done";
    after = [ "devenv:processes:ticker" ];
  };
}
