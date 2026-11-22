# Runs once, inside the sandbox, in confined provisioning.
# Tools `go install` builds land in the project, not in the read-only prefix.
export GOBIN="$PWD/.bin"
export PATH="$GOBIN:$PATH"
