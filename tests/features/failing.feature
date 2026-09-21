Feature: a failure on purpose

  Scenario: the wrong heading
    Given I am on "/login"
    Then the "h1" element should contain "Welcome"
