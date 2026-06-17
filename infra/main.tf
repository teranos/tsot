# Two provider configurations.
#
# - Primary (`aws`): eu-central-1. Where the static site bucket and
#   eventually the Lightsail relay live. Closest to where the operator
#   (Europe-based) does AWS work.
#
# - `aws.us_east_1` alias: us-east-1 only. ACM hard-requires CloudFront
#   certificates to live in us-east-1; this alias exists solely so the
#   certs in `acm.tf` can be issued there without changing the primary
#   region. Don't drop other resources here.

provider "aws" {
  region  = var.aws_region
  profile = var.aws_profile

  default_tags {
    tags = {
      Project   = "roam"
      ManagedBy = "opentofu"
    }
  }
}

provider "aws" {
  alias   = "us_east_1"
  region  = "us-east-1"
  profile = var.aws_profile

  default_tags {
    tags = {
      Project   = "roam"
      ManagedBy = "opentofu"
    }
  }
}
