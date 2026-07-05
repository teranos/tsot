variable "aws_region" {
  description = "Primary AWS region. Lightsail + S3 live here. CloudFront is global; ACM for CF is locked to us-east-1 via the provider alias."
  type        = string
  default     = "eu-central-1"
}

variable "aws_profile" {
  description = "AWS CLI profile to use. Account 548351057127 holds sbvh.nl + the tfstate.sbvh state bucket."
  type        = string
  default     = "sbvh"
}

variable "root_domain" {
  description = "Apex domain managed in Route 53 (must already be a hosted zone)."
  type        = string
  default     = "sbvh.nl"
}

variable "rave_subdomain" {
  description = "Subdomain serving the rave (Bevy + libp2p rave party) wasm bundle through CloudFront."
  type        = string
  default     = "rave"
}

variable "rave_bucket_name" {
  description = "S3 bucket name for rave's dist/ contents. Global namespace; pick something unique."
  type        = string
  default     = "rave-sbvh-static"
}

variable "seer_subdomain" {
  description = "Subdomain serving seer's wasm bundle + per-commit perf reports through CloudFront."
  type        = string
  default     = "seer"
}

variable "seer_bucket_name" {
  description = "S3 bucket name for seer's dist/ + perf reports. Global namespace; pick something unique."
  type        = string
  default     = "seer-sbvh-static"
}

