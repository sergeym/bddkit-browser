Feature: evidence on demand

  Scenario: a screenshot from a passing step
    Given I am on "/login"
    And I am in debug mode
    When I take a screenshot
    Then the "text=Sign in" element should be visible
