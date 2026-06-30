# The hosted zone for sbvh.nl already exists in the SBVH personal
# account. We reference it; we don't manage it. Records created by
# this Terraform stack go alongside whatever else lives in the zone.

data "aws_route53_zone" "root" {
  name         = "${var.root_domain}."
  private_zone = false
}

# A-alias for the game (CloudFront distribution).
resource "aws_route53_record" "game" {
  zone_id = data.aws_route53_zone.root.zone_id
  name    = local.game_fqdn
  type    = "A"

  alias {
    name                   = aws_cloudfront_distribution.static.domain_name
    zone_id                = aws_cloudfront_distribution.static.hosted_zone_id
    evaluate_target_health = false
  }
}

# ACM DNS-validation records — created from the for_each over the
# domain_validation_options that the certs surface. ACM uses DNS-01;
# Route 53 owns the zone, so validation completes within a minute or
# two of records appearing.

resource "aws_route53_record" "game_cert_validation" {
  for_each = {
    for dvo in aws_acm_certificate.game.domain_validation_options : dvo.domain_name => {
      name   = dvo.resource_record_name
      record = dvo.resource_record_value
      type   = dvo.resource_record_type
    }
  }

  zone_id         = data.aws_route53_zone.root.zone_id
  name            = each.value.name
  type            = each.value.type
  records         = [each.value.record]
  ttl             = 60
  allow_overwrite = true
}

