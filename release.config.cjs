// semantic-release config (read by the release job in .github/workflows/release.yml).
//
// The repo has immutable releases on: once a release is published, nothing can be uploaded to
// it. A stable release on main gets its binaries afterwards, from the build jobs, so it starts
// as a draft; the publish job uploads the files and SHA256SUMS, then publishes it. Prereleases
// on dev carry no files and are published right away.
const onMain = process.env.GITHUB_REF === "refs/heads/main";

module.exports = {
  branches: ["main", { name: "dev", prerelease: "dev" }],
  plugins: [
    "@semantic-release/commit-analyzer",
    "@semantic-release/release-notes-generator",
    ["@semantic-release/exec", { prepareCmd: "scripts/set-version.sh ${nextRelease.version}" }],
    [
      "@semantic-release/git",
      {
        assets: ["Cargo.toml", "Cargo.lock"],
        message: "ci(release): ${nextRelease.version}",
      },
    ],
    ["@semantic-release/github", { draftRelease: onMain }],
  ],
};
