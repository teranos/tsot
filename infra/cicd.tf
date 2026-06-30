# GitHub Actions deploy pipeline.
#
# No long-lived AWS access keys in CI. The workflow assumes an IAM
# role via OIDC federation; AWS verifies the GHA-issued OIDC token
# carries the right `sub` claim before issuing temporary STS credentials.
#
# The role's permissions are scoped to exactly what the deploy needs:
#   - read the static bucket's name + write objects
#   - create cloudfront invalidations on the static distribution
# No tofu apply, no Lightsail, no Secrets Manager. CI cannot rotate
# infrastructure — that stays human.

# Shared OIDC provider — one per account, not per repo. If another
# project later adds GHA federation, they reference this same provider
# via `data.aws_iam_openid_connect_provider`.
resource "aws_iam_openid_connect_provider" "github" {
  url             = "https://token.actions.githubusercontent.com"
  client_id_list  = ["sts.amazonaws.com"]
  # GitHub's published OIDC thumbprint. Verifying via
  # https://token.actions.githubusercontent.com/.well-known/openid-configuration
  # is the canonical source; if GitHub rotates, this needs updating.
  thumbprint_list = ["6938fd4d98bab03faadb97b34396831e3780aea1"]
}

variable "github_repo" {
  description = "GitHub repository allowed to assume the deploy role. Format: \"owner/repo\". The trust policy restricts assumption to OIDC tokens whose `sub` claim references this repo's master branch."
  type        = string
  default     = "teranos/tsot"
}

