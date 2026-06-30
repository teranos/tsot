# The hosted zone for sbvh.nl already exists in the SBVH personal
# account. We reference it; we don't manage it. Records created by
# this Terraform stack go alongside whatever else lives in the zone.

data "aws_route53_zone" "root" {
  name         = "${var.root_domain}."
  private_zone = false
}


