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

variable "game_subdomain" {
  description = "Subdomain that serves the static game bundle through CloudFront."
  type        = string
  default     = "roam"
}

variable "relay_subdomain" {
  description = "Subdomain that fronts the libp2p relay through CloudFront."
  type        = string
  default     = "relay"
}

variable "relay_origin_domain" {
  description = <<-EOT
    Domain name CloudFront talks to when forwarding the relay's
    WebSocket. Set this to a domain that resolves to the Lightsail
    public IP (e.g. a separate Route 53 A-record like
    `origin-relay.sbvh.nl → <lightsail-ip>`) once the box is up.
    Placeholder by default so the distribution can be planned + created
    before the Lightsail instance exists.
  EOT
  type        = string
  default     = "origin-relay.sbvh.nl"
}

variable "relay_origin_port" {
  description = "TCP port the relay listens on. Plain WS; CloudFront terminates TLS."
  type        = number
  default     = 9001
}

variable "static_bucket_name" {
  description = "S3 bucket name for the static dist/ contents. Global namespace; pick something unique."
  type        = string
  default     = "roam-sbvh-static"
}

variable "universe_subdomain" {
  description = "Subdomain serving the universe (Bevy) wasm bundle through CloudFront."
  type        = string
  default     = "universe"
}

variable "universe_bucket_name" {
  description = "S3 bucket name for universe's dist/ contents. Global namespace; pick something unique."
  type        = string
  default     = "universe-sbvh-static"
}

variable "universe_deploy_refs" {
  description = "Git refs allowed to assume the universe-github-deploy IAM role via OIDC. Includes the working branch during v0.5 development."
  type        = list(string)
  default     = ["refs/heads/master", "refs/heads/bevy"]
}
