terraform {
  required_version = "= 1.11.6"

  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "= 6.50.0"
    }
  }

  # State lives in the existing `tfstate.sbvh` bucket alongside other
  # SBVH projects (mastodon, state-storage bootstrap). Flat top-level
  # key per the bucket's convention.
  #
  # Backend args do NOT accept variables — they have to be literal.
  # If profile or region ever changes, update this block + run
  # `tofu init -reconfigure`.
  backend "s3" {
    bucket  = "tfstate.sbvh"
    key     = "roam"
    region  = "eu-central-1"
    profile = "sbvh"
    encrypt = true
  }
}
