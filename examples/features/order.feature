Feature: an order placed in the browser reaches the backend

  Background:
    Given I am on "/login"
    And I fill in "email" with "ann@example.test"
    And I fill in "password" with "secret"
    And I press "Sign in"

  Scenario: the id on the page is the order the API knows
    When I follow "New order"
    And I press "Pay"
    And I expect the next assertion to pass within "5" seconds
    Then the browser should have sent a "POST" request to "/api/orders"
    And I expect the next assertion to pass within "5" seconds
    And the last request to "/api/orders" should have status "201"
    And I read the status of the last request to "/api/orders" as "paidStatus"
    And variable "paidStatus" should be equal to "201"
    And the browser console should have no errors
    And I expect the next assertion to pass within "5" seconds
    Then the "[data-test=order-id]" element should be visible
    When I read the "[data-test=order-id]" element text as "orderId"
    And I request "/api/orders/<<orderId>>"
    Then the response code is 200
    And the response body contains JSON:
      """
      {"id": "<<orderId>>", "status": "paid"}
      """
