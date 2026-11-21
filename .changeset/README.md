# Changesets

Every user-facing change needs a changeset: run `npx changeset`, pick the bump, describe the change.
Merging to `main` opens a "chore: version packages" PR; merging that PR tags the release, creates the
GitHub release and attaches `project-tiles-<version>.zip` (the mod folder).
