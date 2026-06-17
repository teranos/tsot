locals {
  game_fqdn  = "${var.game_subdomain}.${var.root_domain}"
  relay_fqdn = "${var.relay_subdomain}.${var.root_domain}"
}
